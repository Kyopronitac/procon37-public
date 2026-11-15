use crate::model::*;
use std::collections::BTreeSet;

#[derive(Clone)]
pub struct Game {
    pub problem: Problem,
    pub agents: Vec<Agent>,
    pub day: usize,
    pub score: Score,
    brands: BTreeSet<usize>,
    traffic_history: Vec<Vec<u32>>,
    observed_traffic: Option<Vec<Traffic>>,
}

pub fn validate_kinds(value: &serde_json::Value, count: usize) -> Result<Vec<u8>, Failure> {
    let kinds: Vec<u8> = serde_json::from_value(value.clone()).map_err(|_| {
        Failure::new(
            "invalid_selection",
            "タイプは0または1の配列で指定してください",
        )
    })?;
    if kinds.len() != count || kinds.iter().any(|&k| k > 1) {
        return Err(Failure::new(
            "invalid_selection",
            "タイプ数または値が不正です",
        ));
    }
    Ok(kinds)
}

impl Game {
    pub fn new(problem: Problem, kinds: &[u8]) -> Self {
        assert_eq!(problem.agents.len(), kinds.len());
        let agents = problem
            .agents
            .iter()
            .zip(kinds)
            .map(|(&pos, &kind)| Agent {
                kind,
                pos,
                fuel: problem.fuel_limits,
            })
            .collect();
        Self {
            problem,
            agents,
            day: 0,
            score: Score::default(),
            brands: BTreeSet::new(),
            traffic_history: Vec::new(),
            observed_traffic: None,
        }
    }
    pub fn traffic(&self) -> Vec<Traffic> {
        self.observed_traffic
            .clone()
            .unwrap_or_else(|| traffic_for(&self.problem, &self.traffic_history, 1))
    }
    /// Use the server's shared road state for the current day, including on clones
    /// used for planning. A successful day consumes it; a rejected plan preserves it.
    /// Future days without an observation continue to use the team's own forecast.
    pub fn set_observed_traffic(&mut self, traffics: Vec<Traffic>) {
        self.observed_traffic = Some(traffics);
    }
    pub fn last_stays(&self) -> &[u32] {
        self.traffic_history
            .last()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    pub fn observation(&self, ends_at: u64) -> Observation {
        Observation {
            ends_at,
            day: self.day,
            agents: self.agents.clone(),
            others: vec![],
            traffics: self.traffic(),
        }
    }
    pub fn waiting_plan(&self) -> Plan {
        vec![vec![-i64::from(self.problem.day_steps[self.day])]; self.agents.len()]
    }

    /// Validate and simulate on local state. A rejected plan cannot partially mutate the game.
    pub fn play_day(&mut self, plan: Plan) -> Result<DayReplay, Failure> {
        self.play_day_with_traffic(plan, self.traffic())
    }
    pub fn play_day_with_traffic(
        &mut self,
        plan: Plan,
        traffics: Vec<Traffic>,
    ) -> Result<DayReplay, Failure> {
        if self.day >= self.problem.day_steps.len() {
            return Err(Failure::new("invalid_plan", "試合は終了しています"));
        }
        let p = &self.problem;
        let n = self.agents.len();
        let steps = p.day_steps[self.day];
        if plan.len() != n {
            return Err(Failure::new(
                "invalid_plan",
                format!("行動計画は{n}台分必要です"),
            ));
        }
        let mut traffic_status = vec![0; p.map.len()];
        for t in &traffics {
            traffic_status[t.pos] = t.status;
        }
        let mut agents = self.agents.clone();
        let mut pending = vec![None; n];
        let mut cursors = vec![0; n];
        let mut stocks = p.spots.iter().map(|s| s.stocks).collect::<Vec<_>>();
        let mut spot_at = vec![None; p.map.len()];
        for (i, s) in p.spots.iter().enumerate() {
            spot_at[s.pos] = Some(i);
        }
        let mut visited = vec![vec![false; p.spots.len()]; n];
        let mut brands = self.brands.clone();
        let mut day_brands = BTreeSet::new();
        let mut udon = self.score.udon;
        let mut counts = vec![0; p.map.len()];
        let mut frames = Vec::with_capacity(steps as usize + 1);
        for step in 0..=steps {
            let mut events = Vec::new();
            if step > 0 {
                for i in 0..n {
                    if let Some(action) = &pending[i] {
                        let action: &Pending = action;
                        if action.until == step && action.fuel_cost > 0 {
                            // The cost is charged when the reserved move is reflected.
                            agents[i].fuel = agents[i]
                                .fuel
                                .checked_sub(action.fuel_cost)
                                .ok_or_else(|| failure(i, step, "燃料が不足しています"))?;
                            events.push(Event::Fuel {
                                agent: i,
                                amount: action.fuel_cost,
                            });
                        }
                    }
                }
                for i in 0..n {
                    if pending[i].as_ref().is_some_and(|a| a.until == step) {
                        if let Some(to) = pending[i].as_ref().unwrap().destination {
                            let from = agents[i].pos;
                            agents[i].pos = to;
                            events.push(Event::Move { agent: i, from, to });
                        }
                        pending[i] = None;
                    }
                }
                for i in 0..n {
                    if agents[i].kind != 0 {
                        continue;
                    }
                    if let Some(s) = spot_at[agents[i].pos]
                        && !visited[i][s]
                        && stocks[s] > 0
                    {
                        stocks[s] -= 1;
                        visited[i][s] = true;
                        udon += 1;
                        brands.insert(p.spots[s].brand);
                        day_brands.insert(p.spots[s].brand);
                        events.push(Event::Collect {
                            agent: i,
                            spot: s,
                            brand: p.spots[s].brand,
                        });
                    }
                }
                let mut supplies = vec![false; p.map.len()];
                for a in &agents {
                    if a.kind == 1 {
                        supplies[a.pos] = true;
                    }
                }
                for (i, a) in agents.iter_mut().enumerate() {
                    if a.kind == 0 && supplies[a.pos] && a.fuel < p.fuel_limits {
                        let amount = p.fuel_limits - a.fuel;
                        a.fuel = p.fuel_limits;
                        events.push(Event::Refill { agent: i, amount });
                    }
                }
                for a in &agents {
                    if p.map.terrain(a.pos) == 1 {
                        counts[a.pos] += 1;
                    }
                }
            }
            for i in 0..n {
                if pending[i].is_some() {
                    continue;
                }
                if step == steps {
                    if cursors[i] != plan[i].len() {
                        return Err(failure(i, step, "日の終了後に未実行の命令があります"));
                    }
                    continue;
                }
                let Some(&action) = plan[i].get(cursors[i]) else {
                    return Err(failure(i, step, "行動計画のステップ数が不足しています"));
                };
                cursors[i] += 1;
                let (duration, destination, cost) = if action < 0 {
                    let duration = action
                        .checked_neg()
                        .and_then(|v| u32::try_from(v).ok())
                        .ok_or_else(|| failure(i, step, "待機ステップ数が大きすぎます"))?;
                    (duration, None, 0)
                } else if action <= 5 {
                    let target = p
                        .map
                        .neighbor(agents[i].pos, action as usize)
                        .filter(|&q| p.map.terrain(q) != 3)
                        .ok_or_else(|| failure(i, step, "池または盤外への移動です"))?;
                    let (duration, cost) = match p.map.terrain(agents[i].pos) {
                        0 => (2, 1),
                        2 => (3, 2),
                        1 => ([1, 2, 4][traffic_status[agents[i].pos] as usize], 2),
                        _ => unreachable!(),
                    };
                    let cost = if agents[i].kind == 1 { 0 } else { cost };
                    // Validate fuel at reflection: the official example's car A
                    // reserves with 1 fuel, then receives supply before arrival.
                    (duration, Some(target), cost)
                } else {
                    return Err(failure(
                        i,
                        step,
                        "移動方向は0〜5、待機は負の整数で指定してください",
                    ));
                };
                if duration > steps - step {
                    return Err(failure(i, step, "行動が日の最終ステップを超えます"));
                }
                pending[i] = Some(Pending {
                    until: step + duration,
                    destination,
                    fuel_cost: cost,
                });
            }
            frames.push(Frame {
                step,
                agents: agents.clone(),
                stocks: stocks.clone(),
                pending: pending.clone(),
                events,
                score: Score {
                    unique_brands: brands.len(),
                    daily_brands: self.score.daily_brands + day_brands.len(),
                    udon,
                },
            });
        }
        self.agents = agents;
        self.brands = brands;
        self.score = frames.last().unwrap().score.clone();
        self.traffic_history.push(counts);
        if self.traffic_history.len() > 2 {
            self.traffic_history.remove(0);
        }
        self.day += 1;
        self.observed_traffic = None;
        Ok(DayReplay {
            day: self.day,
            steps,
            traffics,
            frames,
            plan,
            submitted_answer: None,
            failure: None,
            elapsed_ms: 0,
        })
    }
}

fn failure(agent: usize, step: u32, message: &str) -> Failure {
    Failure {
        kind: "invalid_plan".into(),
        message: message.into(),
        agent: Some(agent),
        step: Some(step),
    }
}

/// Sum before comparing: this avoids any rounding ambiguity at traffic thresholds.
pub fn traffic_for(problem: &Problem, history: &[Vec<u32>], players: u32) -> Vec<Traffic> {
    assert!(players > 0);
    (0..problem.map.len())
        .filter(|&pos| problem.map.terrain(pos) == 1)
        .map(|pos| {
            let count: u64 = history
                .iter()
                .rev()
                .take(2)
                .map(|v| u64::from(v[pos]))
                .sum();
            Traffic {
                pos,
                status: if count >= u64::from(problem.jammed_threshold) * u64::from(players) {
                    2
                } else if count >= u64::from(problem.busy_threshold) * u64::from(players) {
                    1
                } else {
                    0
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn problem(cells: Vec<u8>, positions: Vec<usize>, steps: Vec<u32>) -> Problem {
        Problem {
            starts_at: 0,
            day_seconds: vec![60; steps.len()],
            day_steps: steps,
            map: Map {
                height: 1,
                width: cells.len(),
                cells: vec![cells],
            },
            spots: vec![Spot {
                brand: 0,
                pos: 2,
                stocks: 4,
            }],
            agents: positions,
            fuel_limits: 3,
            players: 1,
            busy_threshold: 1,
            jammed_threshold: 3,
        }
    }
    #[test]
    fn matches_official_supplement_every_final_state_and_traffic() {
        let p = problem(vec![1, 1, 0, 0], vec![0, 0, 0, 0, 2, 3, 3, 2], vec![6]);
        let mut g = Game::new(p, &[0, 0, 0, 0, 0, 0, 0, 1]);
        g.traffic_history.push(vec![0, 1, 0, 0]);
        let day = g
            .play_day(vec![
                vec![2, 2, 2, -1],
                vec![-1, 2, 2, 2],
                vec![-2, 2, 2, -1],
                vec![-3, 2, -2],
                vec![5, 5, -2],
                vec![5, 5, -2],
                vec![-2, 5, 5],
                vec![5, 5, -2],
            ])
            .unwrap();
        assert_eq!(
            g.agents.iter().map(|a| a.pos).collect::<Vec<_>>(),
            vec![3, 3, 2, 1, 0, 1, 1, 0]
        );
        assert_eq!(
            g.agents.iter().map(|a| a.fuel).collect::<Vec<_>>(),
            vec![0, 0, 1, 1, 3, 1, 1, 3]
        );
        assert_eq!(g.traffic_history.last().unwrap()[..2], [12, 17]);
        assert_eq!(
            g.score,
            Score {
                unique_brands: 1,
                daily_brands: 1,
                udon: 4
            }
        );
        assert_eq!(day.frames[0].score.udon, 0);
        assert_eq!(day.frames[1].score.udon, 1);
        assert_eq!(day.frames[1].agents[4].pos, 2);
        assert_eq!(day.frames[1].agents[4].fuel, 3);
        assert_eq!(day.frames[2].agents[5].fuel, 2);
        assert!(day.frames[4].events.contains(&Event::Collect {
            agent: 1,
            spot: 0,
            brand: 0
        }));
        assert!(
            !day.frames[4]
                .events
                .iter()
                .any(|e| matches!(e, Event::Collect { agent: 6, .. }))
        );
    }
    #[test]
    fn rejection_is_atomic_and_checks_final_duration_and_fuel() {
        let mut g = Game::new(problem(vec![0, 0, 0, 0], vec![0], vec![4]), &[0]);
        let initial = g.agents.clone();
        for plan in [
            vec![vec![2]],
            vec![vec![2, 2, 2]],
            vec![vec![5, -2]],
            vec![vec![6]],
            vec![vec![i64::MIN]],
            vec![],
        ] {
            assert!(g.play_day(plan).is_err());
            assert_eq!(g.day, 0);
            assert_eq!(g.agents, initial);
            assert_eq!(g.score.udon, 0);
        }
        g.agents[0].fuel = 0;
        assert!(g.play_day(vec![vec![2, -2]]).is_err());
        g.play_day(vec![vec![-4]]).unwrap();
        assert_eq!(g.agents[0].fuel, 0);
    }
    #[test]
    fn acquisition_resets_daily_and_fuel_is_not_reset() {
        let mut p = problem(vec![0, 0, 0], vec![2], vec![2, 2]);
        p.spots[0].stocks = 1;
        let mut g = Game::new(p, &[0]);
        g.agents[0].fuel = 1;
        g.play_day(vec![vec![-2]]).unwrap();
        g.play_day(vec![vec![-2]]).unwrap();
        assert_eq!(
            g.score,
            Score {
                unique_brands: 1,
                daily_brands: 2,
                udon: 2
            }
        );
        assert_eq!(g.agents[0].fuel, 1);
    }
    #[test]
    fn traffic_uses_previous_two_days_and_is_fixed_during_day() {
        let mut p = problem(vec![1, 0, 0], vec![0], vec![2, 2, 2, 2]);
        p.busy_threshold = 2;
        p.jammed_threshold = 4;
        let mut g = Game::new(p, &[1]);
        assert_eq!(g.traffic()[0].status, 0);
        let d = g.play_day(vec![vec![-2]]).unwrap();
        assert_eq!(d.traffics[0].status, 0);
        assert_eq!(g.traffic()[0].status, 1);
        g.play_day(vec![vec![-2]]).unwrap();
        assert_eq!(g.traffic()[0].status, 2);
        // Move away from a jammed road in a four-step day.
        g.problem.day_steps[2] = 4;
        g.play_day(vec![vec![2]]).unwrap();
        g.play_day(vec![vec![-2]]).unwrap();
        assert_eq!(g.traffic()[0].status, 1); // 3 stays on departure day, then 0.
    }
    #[test]
    fn observed_traffic_controls_moves_and_survives_rejected_plans() {
        let p = problem(vec![1, 0, 0], vec![0], vec![4, 4]);
        let mut g = Game::new(p, &[0]);
        let shared = vec![Traffic { pos: 0, status: 2 }];
        g.set_observed_traffic(shared.clone());
        let initial = g.agents.clone();
        // This would be legal with the team's free-flow forecast, but the shared
        // jam makes the move alone consume the entire day.
        assert!(g.play_day(vec![vec![2, -3]]).is_err());
        assert_eq!(g.day, 0);
        assert_eq!(g.agents, initial);
        assert_eq!(g.traffic(), shared);
        assert!(g.last_stays().is_empty());
        let mut forecast = g.clone();
        let predicted = forecast.play_day(vec![vec![2]]).unwrap();
        let actual = g.play_day(vec![vec![2]]).unwrap();
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(&predicted).unwrap()
        );
        assert_eq!(actual.traffics, shared);
        assert_eq!(actual.frames[3].agents[0].pos, 0);
        assert_eq!(actual.frames[4].agents[0].pos, 1);
        assert_eq!(g.agents[0].fuel, 1);
        assert_eq!(g.day, 1);
        assert_eq!(g.observed_traffic, None);
        // The next server observation replaces the forecast, even when empty.
        g.set_observed_traffic(vec![]);
        assert!(g.traffic().is_empty());
        g.play_day(g.waiting_plan()).unwrap();
        assert_eq!(g.observed_traffic, None);
    }
    #[test]
    fn supply_is_available_before_the_next_reservation_and_at_final_step() {
        let p = problem(vec![0, 0, 0], vec![0, 1], vec![2]);
        let mut g = Game::new(p, &[0, 1]);
        g.agents[0].fuel = 1;
        let d = g.play_day(vec![vec![2], vec![-2]]).unwrap();
        assert_eq!(g.agents[0].fuel, 3);
        assert!(d.frames[2].events.contains(&Event::Refill {
            agent: 0,
            amount: 3
        }));
    }
}
