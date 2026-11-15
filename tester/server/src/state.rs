//! 対戦窓口と実行処理が共有する「進行中の試合」の状態。
//!
//! 対戦窓口はソルバからの提出(車種選択・行動計画)をここへ記録し、
//! 実行処理は試合終了後にこの記録から結果を再現する。

use procon37_common::model::{Agent, Traffic};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// 1回の提出(車種選択または行動計画)。却下された提出も残す。
#[derive(Clone, Debug)]
pub struct Submission {
    /// 提出本文。JSONとして読めない場合は文字列で保持する。
    pub body: Value,
    /// 提出時刻(UNIXミリ秒)。対戦窓口が提出を受け取った時刻で、回答時間の終点に使う。
    pub at_ms: u64,
    /// 公式対戦サーバーのHTTPステータス。
    pub status: u16,
    /// 行動計画に対して返る revision。車種選択では常に None。
    pub revision: Option<i64>,
    pub accepted: bool,
    /// 行動計画が日の締切の前後どちらで処理されたか判断できず、記録した日が推定であること。
    pub uncertain_day: bool,
    /// 公式対戦サーバーの応答を受け取れず、採否が分からないこと。再現時に翌日の状態で確かめる。
    pub unknown: bool,
}

/// 1チームぶんの記録。
#[derive(Clone, Debug, Default)]
pub struct TeamRecord {
    /// 初めて問題(`/setting`)を取得した時刻。
    pub setting_at_ms: Option<u64>,
    pub kinds: Vec<Submission>,
    /// 日(0始まり) -> その日の状態を初めて取得した時刻。回答時間の起点に使う。
    pub day_seen_ms: BTreeMap<usize, u64>,
    /// 日(0始まり) -> その日の行動計画の提出。
    pub plans: BTreeMap<usize, Vec<Submission>>,
    /// 日(0始まり) -> その日の状態で初めて観測した自チームの車。採否が分からない提出を確かめるのに使う。
    pub day_agents: BTreeMap<usize, Vec<Agent>>,
    /// 日(0始まり) -> その日の状態で初めて観測した道路状態(公式対戦サーバーが全チームの滞在から求めた値)。
    pub day_traffics: BTreeMap<usize, Vec<Traffic>>,
}

/// 進行中の1試合。
pub struct ActiveMatch {
    /// 試合ごとに一意なID。プロセスを再起動しても重複しない。
    pub id: String,
    pub tokens: Vec<String>,
    /// チーム番号順の記録。
    pub records: Vec<TeamRecord>,
    /// 公式対戦サーバーの待受ポート(127.0.0.1)。
    pub internal_port: u16,
    /// 車種選択の共通起点(UNIXミリ秒)。公式サーバー起動時に確定し、取得時刻に依存しない。
    pub select_started_ms: u64,
    /// 車種選択の締切(UNIXミリ秒)。
    pub select_ends_ms: u64,
    /// 制限時間を公式条件から短縮しているか。短縮時だけ、車種選択の残り時間を `/setting` の応答ヘッダで知らせる。
    pub shortened: bool,
    /// 各日の秒数。観測した1日の締切から各日の締切を求め、提出の記録先を決めるのに使う。
    pub day_seconds: Vec<u32>,
    /// 直近に観測した状態の日と締切(`endsAt`、UNIX秒)。どのチームの観測でもよい(日程は全チーム共通)。
    pub observed: Option<(usize, u64)>,
    /// 対戦窓口で処理中の提出の数(InFlight)。記録を取り出す前に0になるまで待つ。
    pub in_flight: usize,
}
impl ActiveMatch {
    pub fn team(&self, token: &str) -> Option<usize> {
        self.tokens.iter().position(|t| t == token)
    }
}

#[derive(Default)]
pub struct Shared {
    pub active: Option<ActiveMatch>,
}

pub type SharedState = Arc<Mutex<Shared>>;

/// 試合を対戦窓口に公開する。drop時(中断・失敗を含む)に必ず取り下げる。
pub struct Registration {
    shared: SharedState,
    id: String,
}
impl Registration {
    pub fn new(shared: &SharedState, active: ActiveMatch) -> Self {
        let id = active.id.clone();
        shared.lock().unwrap().active = Some(active);
        Self {
            shared: shared.clone(),
            id,
        }
    }
    /// 対戦窓口で処理中の提出が記録されるまで、最長 `limit` 待つ。
    /// 締切直前に受理された提出は、試合終了を検知した後に記録されることがあるため。
    pub async fn drain(&self, limit: Duration) {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            let busy = self
                .shared
                .lock()
                .unwrap()
                .active
                .as_ref()
                .is_some_and(|a| a.id == self.id && a.in_flight > 0);
            if !busy {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    /// 公開を終え、記録を取り出す。
    pub fn finish(self) -> Vec<TeamRecord> {
        let mut guard = self.shared.lock().unwrap();
        match guard.active.take_if(|a| a.id == self.id) {
            Some(active) => active.records,
            None => vec![],
        }
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.shared.lock() {
            guard.active.take_if(|a| a.id == self.id);
        }
    }
}

/// 対戦窓口で処理中の提出。生きている間は試合の記録を取り出さない(Registration::drain)。
pub struct InFlight {
    shared: SharedState,
    id: String,
}
impl InFlight {
    /// 進行中の試合(`active`)の処理中の提出として数える。呼び出し側が共有状態のロックを持ったまま呼ぶ。
    pub fn enter(shared: &SharedState, active: &mut ActiveMatch) -> Self {
        active.in_flight += 1;
        Self {
            shared: shared.clone(),
            id: active.id.clone(),
        }
    }
}
impl Drop for InFlight {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.shared.lock()
            && let Some(active) = guard.active.as_mut().filter(|a| a.id == self.id)
        {
            active.in_flight = active.in_flight.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_are_taken_after_submissions_in_flight_are_recorded() {
        let shared = SharedState::default();
        let registration = Registration::new(
            &shared,
            ActiveMatch {
                id: "m".into(),
                tokens: vec!["t".into()],
                records: vec![TeamRecord::default()],
                internal_port: 0,
                select_started_ms: 0,
                select_ends_ms: 0,
                shortened: false,
                day_seconds: vec![10],
                observed: None,
                in_flight: 0,
            },
        );
        // 試合終了の検知後に、締切直前に受理された提出が記録される場合。
        let flight = {
            let mut guard = shared.lock().unwrap();
            InFlight::enter(&shared, guard.active.as_mut().unwrap())
        };
        let writer = shared.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let submission = Submission {
                body: Value::Null,
                at_ms: 0,
                status: 200,
                revision: Some(1),
                accepted: true,
                uncertain_day: false,
                unknown: false,
            };
            writer.lock().unwrap().active.as_mut().unwrap().records[0]
                .plans
                .entry(0)
                .or_default()
                .push(submission);
            drop(flight);
        });
        registration.drain(Duration::from_secs(5)).await;
        let records = registration.finish();
        assert_eq!(records[0].plans[&0].len(), 1);
        assert!(shared.lock().unwrap().active.is_none());
    }
}
