use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;

pub const PROTOCOL_VERSION: u32 = 1;
pub const REPLAY_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MatchFormat {
    First,
    Semifinal,
    Final,
}

impl MatchFormat {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "first" => Ok(Self::First),
            "semifinal" => Ok(Self::Semifinal),
            "final" => Ok(Self::Final),
            _ => anyhow::bail!("形式は first / semifinal / final を指定してください"),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Semifinal => "semifinal",
            Self::Final => "final",
        }
    }
    pub fn parameters(self) -> (usize, usize, usize, usize, u64) {
        match self {
            Self::First => (16, 4, 10, 14, 60_000),
            Self::Semifinal => (24, 5, 14, 20, 90_000),
            Self::Final => (32, 7, 20, 28, 120_000),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Map {
    pub height: usize,
    pub width: usize,
    pub cells: Vec<Vec<u8>>,
}
impl Map {
    pub fn len(&self) -> usize {
        self.width * self.height
    }
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
    pub fn terrain(&self, pos: usize) -> u8 {
        self.cells[pos / self.width][pos % self.width]
    }
    pub fn set(&mut self, pos: usize, terrain: u8) {
        self.cells[pos / self.width][pos % self.width] = terrain;
    }
    pub fn neighbor(&self, pos: usize, direction: usize) -> Option<usize> {
        if direction >= 6 || pos >= self.len() {
            return None;
        }
        let (y, x) = ((pos / self.width) as isize, (pos % self.width) as isize);
        let upper_left = if y % 2 == 0 { 0 } else { -1 };
        let offsets = [
            (-1, upper_left),
            (-1, upper_left + 1),
            (0, 1),
            (1, upper_left + 1),
            (1, upper_left),
            (0, -1),
        ];
        let (dy, dx) = offsets[direction];
        let (yy, xx) = (y + dy, x + dx);
        (yy >= 0 && xx >= 0 && yy < self.height as isize && xx < self.width as isize)
            .then_some((yy * self.width as isize + xx) as usize)
    }
    pub fn neighbors(&self, pos: usize) -> impl Iterator<Item = usize> + '_ {
        (0..6).filter_map(move |d| self.neighbor(pos, d))
    }
    pub fn reachable(&self, start: usize) -> Vec<bool> {
        let mut seen = vec![false; self.len()];
        if start >= self.len() || self.terrain(start) == 3 {
            return seen;
        }
        let mut queue = VecDeque::from([start]);
        seen[start] = true;
        while let Some(i) = queue.pop_front() {
            for j in self.neighbors(i) {
                if !seen[j] && self.terrain(j) != 3 {
                    seen[j] = true;
                    queue.push_back(j);
                }
            }
        }
        seen
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Spot {
    pub brand: usize,
    pub pos: usize,
    pub stocks: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub starts_at: u64,
    pub day_seconds: Vec<u32>,
    pub day_steps: Vec<u32>,
    pub map: Map,
    pub spots: Vec<Spot>,
    pub agents: Vec<usize>,
    pub fuel_limits: u32,
    pub players: u32,
    pub busy_threshold: u32,
    pub jammed_threshold: u32,
}
impl Problem {
    pub fn validate(&self) -> Result<()> {
        self.validate_players(1)
    }
    pub fn validate_players(&self, players: u32) -> Result<()> {
        let m = &self.map;
        ensure!(
            (1..=32).contains(&m.width) && (1..=32).contains(&m.height),
            "マップの幅・高さが不正です"
        );
        ensure!(
            m.cells.len() == m.height
                && m.cells
                    .iter()
                    .all(|r| r.len() == m.width && r.iter().all(|&t| t <= 3)),
            "地形配列が不正です"
        );
        ensure!(
            (1..=10).contains(&self.day_steps.len())
                && self.day_seconds.len() == self.day_steps.len(),
            "日数が不正です"
        );
        ensure!(
            self.day_steps.iter().all(|&n| (1..=256).contains(&n))
                && self.day_seconds.iter().all(|&n| n > 0),
            "日別の制限が不正です"
        );
        ensure!(
            (1..=8).contains(&self.agents.len()) && self.players == players && players > 0,
            "プレイヤー数またはエージェント数が不正です"
        );
        ensure!(
            self.fuel_limits >= 2
                && self.busy_threshold > 0
                && self.jammed_threshold > self.busy_threshold,
            "燃料上限または交通量の基準が不正です"
        );
        let mut occupied = vec![false; m.len()];
        ensure!(
            !self.spots.is_empty() && self.spots.len() <= 32,
            "スポット数が不正です"
        );
        for s in &self.spots {
            ensure!(
                s.pos < m.len() && m.terrain(s.pos) == 0 && !occupied[s.pos],
                "スポットの位置が不正です"
            );
            ensure!(
                s.stocks > 0 && s.stocks <= self.agents.len() as u32,
                "スポットの在庫数が不正です"
            );
            occupied[s.pos] = true;
        }
        for &p in &self.agents {
            ensure!(
                p < m.len() && m.terrain(p) == 0 && !occupied[p],
                "初期位置が不正です"
            );
            occupied[p] = true;
        }
        let reachable = m.reachable(self.agents[0]);
        ensure!(
            (0..m.len()).all(|p| m.terrain(p) == 3 || reachable[p]),
            "通行可能セルが連結していません"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Case {
    pub generator_version: String,
    pub seed: String,
    pub format: MatchFormat,
    pub problem: Problem,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Agent {
    pub kind: u8,
    pub pos: usize,
    pub fuel: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Traffic {
    pub pos: usize,
    pub status: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub ends_at: u64,
    pub day: usize,
    pub agents: Vec<Agent>,
    pub others: Vec<Value>,
    pub traffics: Vec<Traffic>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub protocol_version: u32,
    pub request_id: u64,
    pub phase: String,
    pub time_limit_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<Problem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Observation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_accepted: Option<bool>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub request_id: u64,
    pub answer: Value,
}

pub type Plan = Vec<Vec<i64>>;
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Score {
    pub unique_brands: usize,
    pub daily_brands: usize,
    pub udon: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    pub until: u32,
    pub destination: Option<usize>,
    pub fuel_cost: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Move {
        agent: usize,
        from: usize,
        to: usize,
    },
    Collect {
        agent: usize,
        spot: usize,
        brand: usize,
    },
    Refill {
        agent: usize,
        amount: u32,
    },
    Fuel {
        agent: usize,
        amount: u32,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Frame {
    pub step: u32,
    pub agents: Vec<Agent>,
    pub stocks: Vec<u32>,
    pub pending: Vec<Option<Pending>>,
    pub score: Score,
    pub events: Vec<Event>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub kind: String,
    pub message: String,
    pub agent: Option<usize>,
    pub step: Option<u32>,
}
impl Failure {
    pub fn new(kind: &str, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
            agent: None,
            step: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayReplay {
    pub day: usize,
    pub steps: u32,
    pub traffics: Vec<Traffic>,
    pub frames: Vec<Frame>,
    pub plan: Plan,
    pub submitted_answer: Option<Value>,
    pub failure: Option<Failure>,
    pub elapsed_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Replay {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opponents: Vec<Replay>,
    pub schema_version: u32,
    pub case: Case,
    pub kinds: Vec<u8>,
    pub selection_answer: Option<Value>,
    pub selection_failure: Option<Failure>,
    pub selection_ms: u64,
    pub days: Vec<DayReplay>,
    pub score: Score,
    pub answer_time_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hex_neighbors_use_even_rows_shifted_right() {
        let m = Map {
            width: 4,
            height: 4,
            cells: vec![vec![0; 4]; 4],
        };
        assert_eq!(
            (0..6).map(|d| m.neighbor(5, d)).collect::<Vec<_>>(),
            vec![Some(0), Some(1), Some(6), Some(9), Some(8), Some(4)]
        );
        assert_eq!(
            (0..6).map(|d| m.neighbor(9, d)).collect::<Vec<_>>(),
            vec![Some(5), Some(6), Some(10), Some(14), Some(13), Some(8)]
        );
        for p in 0..16 {
            for d in 0..6 {
                if let Some(q) = m.neighbor(p, d) {
                    assert_eq!(m.neighbor(q, (d + 3) % 6), Some(p));
                }
            }
        }
        assert_eq!(m.neighbor(0, 0), None);
        assert_eq!(m.neighbor(0, 5), None);
    }
}
