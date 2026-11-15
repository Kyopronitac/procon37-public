// solver04: 再構築3。出典コミット: 6961e7b1258f11bb2dfcfe0efa28947a8c0f663a
// アルゴリズム・設定・対応表: solver/docs/rebuild-experiment-imak/README.md
// 詳細: solver/docs/rebuild-experiment-imak/solver04/algorithm.md
// 記録用の単一ファイル。開発用の main.rs・solver/*.rs は別途維持する。
// 通信・探索・テストを収録する。tester と serde_json は Cargo 依存。
// 元の外部モジュールをインライン化し、探索処理・定数・テストを保持している。

// 元ファイル: solver/mod.rs
mod solver {
    // 元ファイル: solver/paths.rs
    mod paths {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        use tester::engine::Game;

        pub const MODES: usize = 4;

        #[derive(Clone, Default)]
        pub struct Leg {
            pub time: i32,
            pub fuel: i32,
            pub spots: u32,
        }

        pub struct Paths {
            pub points: Vec<usize>,
            pub starts: Vec<usize>,
            pub initial_masks: Vec<u32>,
            pub legs: Vec<Leg>,
            moves: Vec<Vec<i64>>,
        }

        impl Paths {
            pub fn new(g: &Game) -> Self {
                let map = &g.problem.map;
                let mut points: Vec<_> = g.problem.spots.iter().map(|s| s.pos).collect();
                let mut starts = Vec::new();
                for a in &g.agents {
                    let id = points.iter().position(|&p| p == a.pos).unwrap_or_else(|| {
                        points.push(a.pos);
                        points.len() - 1
                    });
                    starts.push(id);
                }
                let mut status = vec![0; map.len()];
                for t in g.traffic() {
                    status[t.pos] = t.status;
                }
                let costs: Vec<_> = (0..map.len())
                    .map(|p| match map.terrain(p) {
                        0 => (2, 1),
                        1 => ([1, 2, 4][status[p] as usize], 2),
                        2 => (3, 2),
                        _ => (10000, 10000),
                    })
                    .collect();
                let mut at = vec![0u32; map.len()];
                for (i, s) in g.problem.spots.iter().enumerate() {
                    at[s.pos] = 1 << i;
                }
                let n = points.len();
                let mut legs = vec![Leg::default(); MODES * n * n];
                let mut moves = vec![Vec::new(); MODES * n * n];
                for (source, &cell) in points.iter().enumerate() {
                    for mode in 0..MODES {
                        let (tw, fw) = [(100, 1), (100, 35), (100, 100), (1, 100)][mode];
                        let mut dist = vec![i32::MAX; map.len()];
                        let mut prev = vec![(0usize, 0usize); map.len()];
                        let mut heap = BinaryHeap::new();
                        dist[cell] = 0;
                        heap.push(Reverse((0, cell)));
                        while let Some(Reverse((d, p))) = heap.pop() {
                            if d != dist[p] {
                                continue;
                            }
                            let nd = d + costs[p].0 * tw + costs[p].1 * fw;
                            for dir in 0..6 {
                                if let Some(q) = map.neighbor(p, dir)
                                    && map.terrain(q) != 3
                                    && nd < dist[q]
                                {
                                    dist[q] = nd;
                                    prev[q] = (p, dir);
                                    heap.push(Reverse((nd, q)));
                                }
                            }
                        }
                        for (target, &end) in points.iter().enumerate() {
                            let mut leg = Leg::default();
                            let mut commands = Vec::new();
                            let mut p = end;
                            while p != cell {
                                leg.spots |= at[p];
                                let (q, dir) = prev[p];
                                leg.time += costs[q].0;
                                leg.fuel += costs[q].1;
                                commands.push(dir as i64);
                                p = q;
                            }
                            commands.reverse();
                            legs[(source * n + target) * MODES + mode] = leg;
                            moves[(source * n + target) * MODES + mode] = commands;
                        }
                    }
                }
                let initial_masks = g.agents.iter().map(|a| at[a.pos]).collect();
                Self {
                    points,
                    starts,
                    initial_masks,
                    legs,
                    moves,
                }
            }

            #[inline]
            pub fn get(&self, from: usize, to: usize, mode: usize) -> &Leg {
                &self.legs[(from * self.points.len() + to) * MODES + mode]
            }

            pub fn commands(&self, from: usize, to: usize, mode: usize) -> &[i64] {
                &self.moves[(from * self.points.len() + to) * MODES + mode]
            }
        }
    }

    // 元ファイル: solver/planner.rs
    mod planner {
        use super::{
            Rng,
            search::{Context, Tour, optimize},
        };
        use std::time::{Duration, Instant};
        use tester::engine::Game;
        use tester::model::{DayReplay, Event, Plan, Problem};

        #[derive(Default)]
        pub struct Memory {
            pub collected: u32,
            tours: Vec<Tour>,
        }

        impl Memory {
            pub fn observe(&mut self, replay: &DayReplay) {
                add_collected(&mut self.collected, replay);
            }
        }

        fn add_collected(mask: &mut u32, replay: &DayReplay) {
            for frame in &replay.frames {
                for e in &frame.events {
                    if let Event::Collect { spot, .. } = e {
                        *mask |= 1 << spot;
                    }
                }
            }
        }

        struct Trial {
            tours: Vec<Tour>,
            value: f64,
        }

        struct RolloutSettings {
            trial_end: Instant,
            request_end: Instant,
            replay_prefix: bool,
        }

        fn transfer(tours: &[Tour], from: &[u8], to: &[u8]) -> Vec<Tour> {
            let mut permutation: Vec<_> = (0..from.len()).collect();
            let became_supply: Vec<_> = (0..from.len())
                .filter(|&a| from[a] == 0 && to[a] == 1)
                .collect();
            let became_collect: Vec<_> = (0..from.len())
                .filter(|&a| from[a] == 1 && to[a] == 0)
                .collect();
            assert_eq!(became_supply.len(), became_collect.len());
            for (&a, &b) in became_supply.iter().zip(&became_collect) {
                permutation[a] = b;
                permutation[b] = a;
            }
            tours
                .iter()
                .map(|t| {
                    let mut next = vec![Vec::new(); to.len()];
                    for (a, &b) in permutation.iter().enumerate() {
                        next[b] = t.0[a].clone();
                    }
                    Tour(next)
                })
                .collect()
        }

        fn rollout(
            root: &Context<'_>,
            collected: u32,
            seeds: &[Tour],
            days: usize,
            budget: RolloutSettings,
            rng: &mut Rng,
            root_weight: f64,
        ) -> Trial {
            let mut g = root.game.clone();
            let mut mask = collected;
            let mut tours = Vec::new();
            let mut potential = 0.0;
            for d in 0..days {
                let now = Instant::now();
                if now >= budget.request_end {
                    break;
                }
                let left = budget.trial_end.saturating_duration_since(now);
                let weight = if d == 0 { root_weight } else { 1.0 };
                let parts = (days - d - 1) as f64 + weight;
                let until = now + left.mul_f64(weight / parts);
                let owned;
                let ctx = if d == 0 {
                    root
                } else {
                    owned = Context::new(&g, mask);
                    &owned
                };
                let future_weight = [0.05, 0.2, 0.7, 1.5, 3.0][rng.index(5)];
                let (t, evaluated) = if budget.replay_prefix
                    && let Some(t) = seeds.get(d)
                    && let Some(evaluated) = ctx.evaluate(t, None)
                    && evaluated.violation == 0
                {
                    (t.clone(), evaluated)
                } else {
                    let t = optimize(ctx, seeds.get(d), until, rng, future_weight);
                    let evaluated = ctx.evaluate(&t, None).unwrap();
                    (t, evaluated)
                };
                let plan = ctx.materialize(&t);
                potential = evaluated.potential;
                let replay = g.play_day(plan).expect("search schedule is feasible");
                add_collected(&mut mask, &replay);
                tours.push(t);
            }
            let value = if tours.len() == days {
                // Even 10 days with 32 shops and 8 cars cannot reverse these priorities.
                g.score.unique_brands as f64 * 1e9
                    + g.score.daily_brands as f64 * 1e6
                    + g.score.udon as f64 * 100.0
                    + potential
            } else {
                // Incomplete horizons are not comparable with completed rollouts.
                f64::NEG_INFINITY
            };
            if tours.is_empty() {
                tours.push(root.empty());
            }
            Trial { tours, value }
        }

        pub fn select(
            problem: &Problem,
            deadline: Instant,
            rng: &mut Rng,
            memory: &mut Memory,
        ) -> Vec<u8> {
            let n = problem.agents.len();
            let mut masks = Vec::new();
            for m in 0..(1usize << n) {
                if m.count_ones() <= 2
                    && (m.count_ones() as usize) < n
                    && (m == 0 || m.count_ones() as usize <= n / 2)
                {
                    masks.push(m);
                }
            }
            let mut candidates: Vec<(Vec<u8>, Trial)> = Vec::new();
            let initial_budget = deadline.saturating_duration_since(Instant::now()) / 2;
            let weight = |m: usize| match m.count_ones() {
                1 => 100u32,
                2 => 25,
                _ => 10,
            };
            let total_weight: u32 = masks.iter().map(|&m| weight(m)).sum();
            for mask in masks {
                if Instant::now() >= deadline && !candidates.is_empty() {
                    break;
                }
                let kinds: Vec<_> = (0..n).map(|i| ((mask >> i) & 1) as u8).collect();
                let game = Game::new(problem.clone(), &kinds);
                let ctx = Context::new(&game, 0);
                let each = initial_budget.mul_f64(weight(mask) as f64 / total_weight as f64);
                let end = (Instant::now() + each).min(deadline);
                let warm = candidates
                    .iter()
                    .filter(|(k, _)| {
                        k.iter().filter(|&&v| v == 1).count() == mask.count_ones() as usize
                    })
                    .max_by(|a, b| a.1.value.total_cmp(&b.1.value))
                    .map(|(k, t)| transfer(&t.tours, k, &kinds))
                    .unwrap_or_default();
                let trial = rollout(
                    &ctx,
                    0,
                    &warm,
                    problem.day_steps.len(),
                    RolloutSettings {
                        trial_end: end,
                        request_end: deadline,
                        replay_prefix: false,
                    },
                    rng,
                    1.0,
                );
                candidates.push((kinds, trial));
            }
            candidates.sort_by(|a, b| b.1.value.total_cmp(&a.1.value));
            candidates.truncate(6);
            let mut iteration = 0;
            while Instant::now() + Duration::from_millis(20) < deadline {
                let i = if iteration % 4 == 0 {
                    0
                } else {
                    rng.index(candidates.len())
                };
                let game = Game::new(problem.clone(), &candidates[i].0);
                let ctx = Context::new(&game, 0);
                let left = deadline.saturating_duration_since(Instant::now());
                let end = (Instant::now() + left.min(Duration::from_secs(4))).min(deadline);
                let supply_count = candidates[i].0.iter().filter(|&&v| v == 1).count();
                let source = if iteration % 2 == 0 {
                    candidates
                        .iter()
                        .position(|(k, _)| k.iter().filter(|&&v| v == 1).count() == supply_count)
                        .unwrap()
                } else {
                    i
                };
                let warm = transfer(
                    &candidates[source].1.tours,
                    &candidates[source].0,
                    &candidates[i].0,
                );
                let trial = rollout(
                    &ctx,
                    0,
                    &warm,
                    problem.day_steps.len(),
                    RolloutSettings {
                        trial_end: end,
                        request_end: deadline,
                        replay_prefix: false,
                    },
                    rng,
                    1.0,
                );
                if trial.value > candidates[i].1.value {
                    candidates[i].1 = trial;
                }
                candidates.sort_by(|a, b| b.1.value.total_cmp(&a.1.value));
                iteration += 1;
            }
            let (kinds, trial) = candidates.remove(0);
            memory.tours = trial.tours;
            eprintln!("selection kinds={kinds:?} rollout_score={}", trial.value);
            kinds
        }

        pub fn plan(game: &Game, deadline: Instant, rng: &mut Rng, memory: &mut Memory) -> Plan {
            let ctx = Context::new(game, memory.collected);
            let horizon = (game.problem.day_steps.len() - game.day).min(3);
            let left = deadline.saturating_duration_since(Instant::now());
            let initial_end = Instant::now() + left / 8;
            let mut best = rollout(
                &ctx,
                memory.collected,
                &memory.tours,
                horizon,
                RolloutSettings {
                    trial_end: initial_end,
                    request_end: deadline,
                    replay_prefix: true,
                },
                rng,
                2.0,
            );
            let mut trials = 1;
            while Instant::now() + Duration::from_millis(20) < deadline {
                let left = deadline.saturating_duration_since(Instant::now());
                let end = (Instant::now() + left.min(Duration::from_millis(1800))).min(deadline);
                let seeds = if trials % 5 == 0 {
                    &[][..]
                } else {
                    &best.tours[..]
                };
                let next = rollout(
                    &ctx,
                    memory.collected,
                    seeds,
                    horizon,
                    RolloutSettings {
                        trial_end: end,
                        request_end: deadline,
                        replay_prefix: false,
                    },
                    rng,
                    2.0,
                );
                if next.value > best.value {
                    best = next;
                }
                trials += 1;
            }
            let plan = ctx.materialize(&best.tours[0]);
            let e = ctx.evaluate(&best.tours[0], None).unwrap();
            eprintln!(
                "day={} candidates={trials} daily={} udon={} potential={:.2}",
                game.day, e.daily, e.udon, e.potential
            );
            memory.tours = best.tours.into_iter().skip(1).collect();
            plan
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn cached_prefix_is_replayed_before_searching_the_remaining_day() {
                let mut problem: Problem = serde_json::from_str(include_str!(
                    "../../../official/sample_maps/sample_16.json"
                ))
                .unwrap();
                problem.players = 1;
                let game = Game::new(problem, &[0, 1, 0, 0]);
                let mut expected = game.clone();
                let mut seeds = Vec::new();
                for _ in 0..2 {
                    let ctx = Context::new(&expected, 0);
                    let point = (0..expected.problem.spots.len())
                        .min_by_key(|&p| ctx.paths.get(ctx.paths.starts[0], p, 0).time)
                        .unwrap();
                    let mut tour = ctx.empty();
                    tour.0[0].push(super::super::search::Stop {
                        point,
                        mode: 0,
                        meeting: 0,
                    });
                    expected.play_day(ctx.materialize(&tour)).unwrap();
                    seeds.push(tour);
                }
                let ctx = Context::new(&game, 0);
                let now = Instant::now();
                let trial = rollout(
                    &ctx,
                    0,
                    &seeds,
                    3,
                    RolloutSettings {
                        trial_end: now + Duration::from_millis(20),
                        request_end: now + Duration::from_secs(1),
                        replay_prefix: true,
                    },
                    &mut Rng::new(0),
                    2.0,
                );
                assert_eq!(trial.tours.len(), 3);
                assert_eq!(&trial.tours[..2], seeds.as_slice());
                assert!(trial.value.is_finite());
                let mut actual = game.clone();
                for tour in &trial.tours[..2] {
                    let ctx = Context::new(&actual, 0);
                    actual.play_day(ctx.materialize(tour)).unwrap();
                }
                assert_eq!(actual.score, expected.score);
                assert_eq!(actual.agents, expected.agents);
                assert_eq!(actual.traffic(), expected.traffic());
            }

            #[test]
            fn expired_rollout_is_not_scored_and_keeps_a_valid_plan() {
                let mut problem: Problem = serde_json::from_str(include_str!(
                    "../../../official/sample_maps/sample_16.json"
                ))
                .unwrap();
                problem.players = 1;
                let game = Game::new(problem, &[0, 1, 0, 0]);
                let ctx = Context::new(&game, 0);
                let expired = Instant::now() - Duration::from_millis(1);
                let trial = rollout(
                    &ctx,
                    0,
                    &[],
                    game.problem.day_steps.len(),
                    RolloutSettings {
                        trial_end: expired,
                        request_end: expired,
                        replay_prefix: false,
                    },
                    &mut Rng::new(0),
                    1.0,
                );
                assert_eq!(trial.value, f64::NEG_INFINITY);
                assert_eq!(trial.tours.len(), 1);
                let mut checked = game.clone();
                checked.play_day(ctx.materialize(&trial.tours[0])).unwrap();
            }

            #[test]
            fn transferring_roles_preserves_the_simulated_routes() {
                let mut problem: Problem = serde_json::from_str(include_str!(
                    "../../../official/sample_maps/sample_16.json"
                ))
                .unwrap();
                problem.players = 1;
                let original = Game::new(problem, &[0, 1, 0, 0]);
                let ctx = Context::new(&original, 0);
                let mut rng = Rng::new(7);
                let tour = optimize(
                    &ctx,
                    None,
                    Instant::now() + Duration::from_millis(40),
                    &mut rng,
                    0.2,
                );
                let mut expected = original.clone();
                expected.play_day(ctx.materialize(&tour)).unwrap();
                let moved = transfer(&[tour], &[0, 1, 0, 0], &[1, 0, 0, 0]);
                let mut actual = original.clone();
                actual.agents.swap(0, 1);
                let new_context = Context::new(&actual, 0);
                let plan = new_context.materialize(&moved[0]);
                actual.play_day(plan).unwrap();
                assert_eq!(actual.score, expected.score);
                assert_eq!(actual.traffic(), expected.traffic());
                expected.agents.swap(0, 1);
                assert_eq!(actual.agents, expected.agents);
            }
        }
    }

    // 元ファイル: solver/search.rs
    mod search {
        use super::{Rng, paths::Paths};
        use std::time::Instant;
        use tester::engine::Game;
        use tester::model::Plan;

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Stop {
            pub point: usize,
            pub mode: usize,
            pub meeting: u8,
        }

        #[derive(Debug, PartialEq, Eq)]
        pub struct Tour(pub Vec<Vec<Stop>>);

        impl Clone for Tour {
            fn clone(&self) -> Self {
                Self(self.0.clone())
            }
            fn clone_from(&mut self, source: &Self) {
                self.0.clone_from(&source.0);
            }
        }

        pub struct Context<'a> {
            pub game: &'a Game,
            pub paths: Paths,
            pub collectors: Vec<usize>,
            pub suppliers: Vec<usize>,
            brands: Vec<u32>,
            stocks: [u32; 8],
            brand_lut: [[u32; 256]; 4],
            seen: u32,
            steps: i32,
            #[cfg(target_arch = "x86_64")]
            hardware_popcount: bool,
        }

        #[derive(Clone, Debug, PartialEq)]
        pub struct Evaluation {
            pub reward: f64,
            pub violation: i32,
            pub unique: u32,
            pub daily: u32,
            pub udon: u32,
            pub potential: f64,
            times: [i32; 8],
            fuel: [i32; 8],
            points: [usize; 8],
            masks: [u32; 8],
        }

        impl Evaluation {
            pub fn value(&self, penalty: f64) -> f64 {
                self.reward - self.violation as f64 * penalty
            }
            pub fn rank(&self) -> f64 {
                self.unique as f64 * 1e8
                    + self.daily as f64 * 1e5
                    + self.udon as f64 * 100.0
                    + self.potential
            }
        }

        impl<'a> Context<'a> {
            pub fn new(game: &'a Game, collected: u32) -> Self {
                let mut ids = Vec::new();
                let mut brands = Vec::new();
                let mut seen = 0;
                for (i, s) in game.problem.spots.iter().enumerate() {
                    let b = ids.iter().position(|&x| x == s.brand).unwrap_or_else(|| {
                        ids.push(s.brand);
                        ids.len() - 1
                    });
                    brands.push(1 << b);
                    if collected & (1 << i) != 0 {
                        seen |= 1 << b;
                    }
                }
                let mut stocks = [0; 8];
                for (s, spot) in game.problem.spots.iter().enumerate() {
                    for mask in stocks.iter_mut().take(spot.stocks as usize) {
                        *mask |= 1 << s;
                    }
                }
                let mut brand_lut = [[0; 256]; 4];
                for (chunk, table) in brand_lut.iter_mut().enumerate() {
                    for bits in 1..256usize {
                        let b = bits.trailing_zeros() as usize;
                        table[bits] = table[bits & (bits - 1)]
                            | brands.get(chunk * 8 + b).copied().unwrap_or(0);
                    }
                }
                Self {
                    game,
                    paths: Paths::new(game),
                    brands,
                    stocks,
                    brand_lut,
                    seen,
                    steps: game.problem.day_steps[game.day] as i32,
                    #[cfg(target_arch = "x86_64")]
                    hardware_popcount: std::is_x86_feature_detected!("popcnt"),
                    collectors: (0..game.agents.len())
                        .filter(|&i| game.agents[i].kind == 0)
                        .collect(),
                    suppliers: (0..game.agents.len())
                        .filter(|&i| game.agents[i].kind == 1)
                        .collect(),
                }
            }

            pub fn empty(&self) -> Tour {
                Tour(vec![Vec::new(); self.game.agents.len()])
            }

            fn travel(&self, e: &mut Evaluation, a: usize, s: Stop, mut output: Option<&mut Plan>) {
                let from = e.points[a];
                let leg = self.paths.get(from, s.point, s.mode);
                e.times[a] += leg.time;
                if self.game.agents[a].kind == 0 {
                    e.fuel[a] -= leg.fuel;
                    if e.fuel[a] < 0 {
                        e.violation += -e.fuel[a];
                        e.fuel[a] = 0;
                    }
                    e.masks[a] |= leg.spots;
                }
                e.points[a] = s.point;
                if let Some(plan) = output.as_mut() {
                    plan[a].extend_from_slice(self.paths.commands(from, s.point, s.mode));
                }
            }

            #[inline(always)]
            pub fn evaluate(
                &self,
                tour: &Tour,
                mut output: Option<&mut Plan>,
            ) -> Option<Evaluation> {
                let n = self.game.agents.len();
                let mut e = Evaluation {
                    reward: 0.0,
                    violation: 0,
                    unique: 0,
                    daily: 0,
                    udon: 0,
                    potential: 0.0,
                    times: [0; 8],
                    fuel: [0; 8],
                    points: [0; 8],
                    masks: [0; 8],
                };
                for a in 0..n {
                    e.fuel[a] = self.game.agents[a].fuel as i32;
                    e.points[a] = self.paths.starts[a];
                    if self.game.agents[a].kind == 0 {
                        e.masks[a] = self.paths.initial_masks[a];
                    }
                }
                let mut heads = [0usize; 8];
                let mut remaining: usize = tour.0.iter().map(Vec::len).sum();
                while remaining > 0 {
                    let mut progressed = false;
                    for a in 0..n {
                        let Some(&s) = tour.0[a].get(heads[a]) else {
                            continue;
                        };
                        if s.meeting == 0 {
                            self.travel(&mut e, a, s, output.as_deref_mut());
                            heads[a] += 1;
                            remaining -= 1;
                            progressed = true;
                        } else {
                            let peer = (0..n).find(|&b| {
                                b != a
                                    && tour.0[b]
                                        .get(heads[b])
                                        .is_some_and(|t| t.meeting == s.meeting)
                            });
                            let Some(b) = peer else { continue };
                            let other = tour.0[b][heads[b]];
                            if s.point != other.point
                                || self.game.agents[a].kind == self.game.agents[b].kind
                            {
                                return None;
                            }
                            self.travel(&mut e, a, s, output.as_deref_mut());
                            self.travel(&mut e, b, other, output.as_deref_mut());
                            let t = e.times[a].max(e.times[b]).max(1);
                            for v in [a, b] {
                                if let Some(out) = output.as_deref_mut()
                                    && t > e.times[v]
                                {
                                    out[v].push((e.times[v] - t) as i64);
                                }
                                e.times[v] = t;
                                if self.game.agents[v].kind == 0 {
                                    e.fuel[v] = self.game.problem.fuel_limits as i32;
                                }
                                heads[v] += 1;
                            }
                            remaining -= 2;
                            progressed = true;
                        }
                    }
                    if !progressed {
                        return None;
                    }
                }
                let mut levels = [0u32; 8];
                for (j, &a) in self.collectors.iter().enumerate() {
                    let mask = e.masks[a];
                    for k in (1..=j).rev() {
                        levels[k] |= levels[k - 1] & mask;
                    }
                    levels[0] |= mask;
                }
                e.udon = levels
                    .iter()
                    .zip(self.stocks)
                    .map(|(&visited, stock)| (visited & stock).count_ones())
                    .sum();
                let visited = levels[0];
                let brands = self.brand_lut[0][(visited & 255) as usize]
                    | self.brand_lut[1][((visited >> 8) & 255) as usize]
                    | self.brand_lut[2][((visited >> 16) & 255) as usize]
                    | self.brand_lut[3][(visited >> 24) as usize];
                e.daily = brands.count_ones();
                e.unique = (brands & !self.seen).count_ones();
                for a in 0..n {
                    e.violation += (e.times[a] - self.steps).max(0);
                    if let Some(out) = output.as_deref_mut()
                        && e.times[a] < self.steps
                    {
                        out[a].push((e.times[a] - self.steps) as i64);
                    }
                }
                if self.game.day + 1 < self.game.problem.day_steps.len() {
                    let capacity = self.game.problem.fuel_limits as f64;
                    for &a in &self.collectors {
                        let fuel = e.fuel[a] as f64;
                        let near = self
                            .suppliers
                            .iter()
                            .map(|&b| self.paths.get(e.points[b], e.points[a], 0).time)
                            .min()
                            .unwrap_or(1000) as f64;
                        e.potential +=
                            5.0 * (fuel / capacity).sqrt() - (near - fuel * 0.75).max(0.0) * 0.07;
                    }
                }
                e.reward = e.unique as f64 * 1000.0
                    + e.daily as f64 * 100.0
                    + e.udon as f64
                    + e.potential * 0.12;
                Some(e)
            }

            pub fn materialize(&self, tour: &Tour) -> Plan {
                let mut plan = vec![Vec::new(); self.game.agents.len()];
                let e = self
                    .evaluate(tour, Some(&mut plan))
                    .expect("acyclic meeting schedule");
                assert_eq!(e.violation, 0);
                plan
            }

            pub fn mutate(&self, t: &mut Tour, rng: &mut Rng) {
                if self.collectors.is_empty() {
                    return;
                }
                let op = rng.index(100);
                let a = self.collectors[rng.index(self.collectors.len())];
                let spots = self.game.problem.spots.len();
                if op < 24 {
                    if t.0[a].len() >= 40 {
                        return;
                    }
                    let point = rng.index(spots);
                    if t.0[a].iter().any(|s| s.point == point) {
                        return;
                    }
                    let mode = rng.index(4);
                    let at = if rng.index(2) == 0 {
                        (0..=t.0[a].len())
                            .min_by_key(|&i| {
                                let before = if i == 0 {
                                    self.paths.starts[a]
                                } else {
                                    t.0[a][i - 1].point
                                };
                                let first = self.paths.get(before, point, mode).time;
                                if let Some(after) = t.0[a].get(i) {
                                    first + self.paths.get(point, after.point, after.mode).time
                                        - self.paths.get(before, after.point, after.mode).time
                                } else {
                                    first
                                }
                            })
                            .unwrap()
                    } else {
                        rng.index(t.0[a].len() + 1)
                    };
                    t.0[a].insert(
                        at,
                        Stop {
                            point,
                            mode,
                            meeting: 0,
                        },
                    );
                } else if op < 36 {
                    if t.0[a].is_empty() {
                        return;
                    }
                    let i = rng.index(t.0[a].len());
                    let s = t.0[a].remove(i);
                    if s.meeting != 0 {
                        for r in &mut t.0 {
                            r.retain(|q| q.meeting != s.meeting);
                        }
                    }
                } else if op < 55 {
                    let a = rng.index(t.0.len());
                    if t.0[a].is_empty() {
                        return;
                    }
                    let i = rng.index(t.0[a].len());
                    let s = t.0[a].remove(i);
                    let b = if s.meeting != 0 || rng.index(3) != 0 {
                        a
                    } else {
                        self.collectors[rng.index(self.collectors.len())]
                    };
                    let j = rng.index(t.0[b].len() + 1);
                    t.0[b].insert(j, s);
                } else if op < 64 {
                    let b = self.collectors[rng.index(self.collectors.len())];
                    if t.0[a].is_empty() || t.0[b].is_empty() {
                        return;
                    }
                    let i = rng.index(t.0[a].len());
                    let j = rng.index(t.0[b].len());
                    if a == b {
                        t.0[a].swap(i, j);
                    } else if t.0[a][i].meeting == 0 && t.0[b][j].meeting == 0 {
                        let s = t.0[a][i];
                        t.0[a][i] = t.0[b][j];
                        t.0[b][j] = s;
                    }
                } else if op < 71 {
                    let a = rng.index(t.0.len());
                    if t.0[a].is_empty() {
                        return;
                    }
                    let i = rng.index(t.0[a].len());
                    let j = rng.index(t.0[a].len());
                    t.0[a][i.min(j)..=i.max(j)].reverse();
                } else if op < 79 {
                    let a = rng.index(t.0.len());
                    if t.0[a].is_empty() {
                        return;
                    }
                    let i = rng.index(t.0[a].len());
                    t.0[a][i].mode = rng.index(4);
                } else if op < 88 && !self.suppliers.is_empty() {
                    let b = self.suppliers[rng.index(self.suppliers.len())];
                    let used = t.0.iter().flatten().fold(1u32, |m, s| m | (1 << s.meeting));
                    let id = (!used).trailing_zeros();
                    if id >= 24 || t.0[a].iter().filter(|s| s.meeting != 0).count() >= 4 {
                        return;
                    }
                    let i = rng.index(t.0[a].len() + 1);
                    let j = rng.index(t.0[b].len() + 1);
                    let point = if !t.0[a].is_empty() && rng.index(2) == 0 {
                        t.0[a][rng.index(t.0[a].len())].point
                    } else {
                        rng.index(spots)
                    };
                    let s = Stop {
                        point,
                        mode: 0,
                        meeting: id as u8,
                    };
                    t.0[a].insert(i, s);
                    t.0[b].insert(j, s);
                } else if op < 94 && !self.suppliers.is_empty() {
                    let b = self.suppliers[rng.index(self.suppliers.len())];
                    if t.0[b].is_empty() {
                        return;
                    }
                    let id = t.0[b][rng.index(t.0[b].len())].meeting;
                    let point = rng.index(spots);
                    for r in &mut t.0 {
                        for s in r {
                            if s.meeting == id {
                                s.point = point;
                            }
                        }
                    }
                } else if op < 98 {
                    if t.0[a].is_empty() {
                        return;
                    }
                    let i = rng.index(t.0[a].len());
                    if t.0[a][i].meeting == 0 {
                        let old = t.0[a][i].point;
                        let point = if rng.index(2) == 0 {
                            let alternatives: Vec<_> = (0..spots)
                                .filter(|&p| self.brands[p] == self.brands[old])
                                .collect();
                            alternatives[rng.index(alternatives.len())]
                        } else {
                            rng.index(spots)
                        };
                        t.0[a][i].point = point;
                    }
                } else {
                    let b = self.collectors[rng.index(self.collectors.len())];
                    if a == b {
                        return;
                    }
                    let i = rng.index(t.0[a].len() + 1);
                    let j = rng.index(t.0[b].len() + 1);
                    let ta = t.0[a].split_off(i);
                    let tb = t.0[b].split_off(j);
                    t.0[a].extend(tb);
                    t.0[b].extend(ta);
                }
            }
        }

        fn polish(
            ctx: &Context<'_>,
            tour: Tour,
            evaluation: Evaluation,
            deadline: Instant,
            future_weight: f64,
        ) -> Tour {
            let quality = |e: &Evaluation| {
                e.rank() + e.potential * (future_weight * 100.0 - 1.0)
                    - e.times.iter().sum::<i32>() as f64 * 0.0001
            };
            let mut best = tour;
            let mut best_eval = evaluation;
            let mut candidate = best.clone();
            let mut checks = 0usize;
            macro_rules! offer {
                () => {{
                    checks += 1;
                    if checks.is_multiple_of(64) && Instant::now() >= deadline {
                        return best;
                    }
                    if let Some(e) = ctx.evaluate(&candidate, None)
                        && e.violation == 0
                        && quality(&e) > quality(&best_eval) + 1e-7
                    {
                        best.clone_from(&candidate);
                        best_eval = e;
                    }
                }};
            }
            while Instant::now() < deadline {
                let base = best.clone();
                let base_eval = best_eval.clone();
                let previous = quality(&base_eval);
                for &a in &ctx.collectors {
                    if base.0[a].len() >= 40 {
                        continue;
                    }
                    for point in 0..ctx.game.problem.spots.len() {
                        if base_eval.masks[a] & (1 << point) != 0 {
                            continue;
                        }
                        for i in 0..=base.0[a].len() {
                            for mode in 0..4 {
                                candidate.clone_from(&base);
                                candidate.0[a].insert(
                                    i,
                                    Stop {
                                        point,
                                        mode,
                                        meeting: 0,
                                    },
                                );
                                offer!();
                            }
                        }
                    }
                }
                for a in 0..base.0.len() {
                    for i in 0..base.0[a].len() {
                        candidate.clone_from(&base);
                        let removed = candidate.0[a].remove(i);
                        if removed.meeting != 0 {
                            for row in &mut candidate.0 {
                                row.retain(|s| s.meeting != removed.meeting);
                            }
                        }
                        offer!();
                        for mode in 0..4 {
                            if mode == base.0[a][i].mode {
                                continue;
                            }
                            candidate.clone_from(&base);
                            candidate.0[a][i].mode = mode;
                            offer!();
                        }
                        for j in i + 1..base.0[a].len() {
                            candidate.clone_from(&base);
                            candidate.0[a][i..=j].reverse();
                            offer!();
                        }
                    }
                }
                for &a in &ctx.collectors {
                    for (i, &stop) in base.0[a].iter().enumerate() {
                        if stop.meeting != 0 {
                            continue;
                        }
                        for &b in &ctx.collectors {
                            if a != b && base.0[b].len() >= 40 {
                                continue;
                            }
                            let len = base.0[b].len() - usize::from(a == b);
                            for j in 0..=len {
                                if a == b && i == j {
                                    continue;
                                }
                                candidate.clone_from(&base);
                                candidate.0[a].remove(i);
                                candidate.0[b].insert(j, stop);
                                offer!();
                            }
                        }
                    }
                }
                for &a in &ctx.suppliers {
                    for stop in &base.0[a] {
                        if stop.meeting == 0 {
                            continue;
                        }
                        for point in 0..ctx.game.problem.spots.len() {
                            if point == stop.point {
                                continue;
                            }
                            candidate.clone_from(&base);
                            for row in &mut candidate.0 {
                                for s in row {
                                    if s.meeting == stop.meeting {
                                        s.point = point;
                                    }
                                }
                            }
                            offer!();
                        }
                    }
                }
                if quality(&best_eval) <= previous + 1e-7 {
                    break;
                }
            }
            best
        }

        pub fn optimize(
            ctx: &Context<'_>,
            seed: Option<&Tour>,
            deadline: Instant,
            rng: &mut Rng,
            future_weight: f64,
        ) -> Tour {
            #[cfg(target_arch = "x86_64")]
            if ctx.hardware_popcount {
                // Dispatch once per search, so scoring remains inline in the hot loop.
                return unsafe { optimize_popcount(ctx, seed, deadline, rng, future_weight) };
            }
            optimize_inner(ctx, seed, deadline, rng, future_weight)
        }

        #[cfg(target_arch = "x86_64")]
        #[target_feature(enable = "popcnt")]
        unsafe fn optimize_popcount(
            ctx: &Context<'_>,
            seed: Option<&Tour>,
            deadline: Instant,
            rng: &mut Rng,
            future_weight: f64,
        ) -> Tour {
            optimize_inner(ctx, seed, deadline, rng, future_weight)
        }

        #[inline(always)]
        fn optimize_inner(
            ctx: &Context<'_>,
            seed: Option<&Tour>,
            deadline: Instant,
            rng: &mut Rng,
            future_weight: f64,
        ) -> Tour {
            let rank = |e: &Evaluation| e.rank() + e.potential * (future_weight * 100.0 - 1.0);
            let brand_count = ctx.brands.iter().fold(0u32, |m, &b| m | b).count_ones();
            let empty = ctx.empty();
            let mut best = empty.clone();
            let mut best_eval = ctx.evaluate(&best, None).unwrap();
            let mut current = seed.cloned().unwrap_or_else(|| empty.clone());
            let mut eval = ctx.evaluate(&current, None).unwrap_or_else(|| {
                current = empty.clone();
                best_eval.clone()
            });
            if eval.violation == 0 && rank(&eval) > rank(&best_eval) {
                best = current.clone();
                best_eval = eval.clone();
            }
            let started = Instant::now();
            let anneal_deadline =
                started + deadline.saturating_duration_since(started).mul_f64(0.88);
            let duration = anneal_deadline
                .saturating_duration_since(started)
                .as_secs_f64()
                .max(0.001);
            let mut iteration = 0u64;
            let mut temp = 8.0;
            let mut penalty = 20.0;
            let mut cycle = 0u64;
            let mut candidate = current.clone();
            loop {
                if iteration.is_multiple_of(128) {
                    let now = Instant::now();
                    if now >= anneal_deadline {
                        break;
                    }
                    let progress = now.duration_since(started).as_secs_f64() / duration;
                    let new_cycle = (progress * 8.0) as u64;
                    if new_cycle != cycle {
                        cycle = new_cycle;
                        current.clone_from(&best);
                        eval = best_eval.clone();
                    }
                    let phase = (progress * 8.0).fract();
                    let peak = if best_eval.daily == brand_count {
                        8.0
                    } else {
                        25.0
                    };
                    temp = peak * (0.008_f64).powf(phase);
                    penalty = 15.0 + 85.0 * phase;
                }
                iteration += 1;
                candidate.clone_from(&current);
                ctx.mutate(&mut candidate, rng);
                let Some(next) = ctx.evaluate(&candidate, None) else {
                    continue;
                };
                if next.violation == 0 && rank(&next) > rank(&best_eval) + 1e-7 {
                    best.clone_from(&candidate);
                    best_eval = next.clone();
                }
                let delta = next.value(penalty) - eval.value(penalty)
                    + (future_weight - 0.12) * (next.potential - eval.potential);
                if delta >= 0.0 || rng.unit() < (delta / temp).exp() {
                    std::mem::swap(&mut current, &mut candidate);
                    eval = next;
                }
            }
            polish(ctx, best, best_eval, deadline, future_weight)
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use std::time::Duration;
            use tester::model::Problem;

            #[cfg(target_arch = "x86_64")]
            #[test]
            fn hardware_population_count_matches_portable_evaluation() {
                let mut p: Problem = serde_json::from_str(include_str!(
                    "../../../official/sample_maps/sample_16.json"
                ))
                .unwrap();
                p.players = 1;
                let game = Game::new(p, &[0, 0, 0, 1]);
                let native = Context::new(&game, 0);
                if !native.hardware_popcount {
                    return;
                }
                #[target_feature(enable = "popcnt")]
                unsafe fn accelerated(ctx: &Context<'_>, tour: &Tour) -> Option<Evaluation> {
                    ctx.evaluate(tour, None)
                }
                let mut portable = Context::new(&game, 0);
                portable.hardware_popcount = false;
                let mut rng = Rng::new(37);
                let mut tour = native.empty();
                for _ in 0..2000 {
                    native.mutate(&mut tour, &mut rng);
                    let expected = portable.evaluate(&tour, None);
                    assert_eq!(unsafe { accelerated(&native, &tour) }, expected);
                    if expected.is_none() {
                        tour = native.empty();
                    }
                }
            }

            #[test]
            fn polishing_preserves_legality_and_does_not_lose_final_day_score() {
                let mut p: Problem = serde_json::from_str(include_str!(
                    "../../../official/sample_maps/sample_16.json"
                ))
                .unwrap();
                p.players = 1;
                let mut game = Game::new(p, &[0, 0, 0, 1]);
                game.day = game.problem.day_steps.len() - 1;
                let ctx = Context::new(&game, 0);
                let mut tour = ctx.empty();
                for &a in &ctx.collectors {
                    for point in 0..game.problem.spots.len() {
                        let mut candidate = tour.clone();
                        candidate.0[a].push(Stop {
                            point,
                            mode: 0,
                            meeting: 0,
                        });
                        if ctx.evaluate(&candidate, None).unwrap().violation == 0 {
                            tour = candidate;
                            break;
                        }
                    }
                }
                let mut before = game.clone();
                before.play_day(ctx.materialize(&tour)).unwrap();
                assert!(before.score.udon > 0);
                let e = ctx.evaluate(&tour, None).unwrap();
                let improved = polish(
                    &ctx,
                    tour,
                    e,
                    Instant::now() + Duration::from_millis(30),
                    0.0,
                );
                let mut after = game.clone();
                after.play_day(ctx.materialize(&improved)).unwrap();
                let key = |g: &Game| (g.score.unique_brands, g.score.daily_brands, g.score.udon);
                assert!(key(&after) >= key(&before));
            }
        }
    }

    #[cfg(test)]
    // 元ファイル: solver/tests.rs
    mod tests {
        use super::{
            Rng,
            search::{Context, Stop, Tour},
        };
        use tester::engine::Game;
        use tester::model::{Event, Map, Problem};

        #[test]
        fn randomly_changed_schedules_agree_with_independent_simulator() {
            let mut problem: Problem =
                serde_json::from_str(include_str!("../../../official/sample_maps/sample_16.json"))
                    .unwrap();
            problem.players = 1;
            let mut g = Game::new(problem, &[0, 0, 0, 1]);
            let mut rng = Rng::new(17);
            let mut collected = 0;
            for _ in 0..g.problem.day_steps.len() {
                let ctx = Context::new(&g, collected);
                let mut current = ctx.empty();
                let mut verified = 0;
                for _ in 0..6000 {
                    let mut next = current.clone();
                    ctx.mutate(&mut next, &mut rng);
                    let Some(e) = ctx.evaluate(&next, None) else {
                        continue;
                    };
                    if e.violation != 0 {
                        continue;
                    }
                    let mut reference = g.clone();
                    reference
                        .play_day(ctx.materialize(&next))
                        .expect("feasible route must be accepted");
                    assert_eq!(
                        reference.score.unique_brands - g.score.unique_brands,
                        e.unique as usize
                    );
                    assert_eq!(
                        reference.score.daily_brands - g.score.daily_brands,
                        e.daily as usize
                    );
                    assert_eq!(reference.score.udon - g.score.udon, e.udon);
                    verified += 1;
                    if rng.index(3) == 0 || e.rank() > ctx.evaluate(&current, None).unwrap().rank()
                    {
                        current = next;
                    }
                }
                assert!(verified > 100);
                let replay = g.play_day(ctx.materialize(&current)).unwrap();
                for frame in replay.frames {
                    for event in frame.events {
                        if let Event::Collect { spot, .. } = event {
                            collected |= 1 << spot;
                        }
                    }
                }
            }
        }

        #[test]
        fn zero_time_rendezvous_waits_before_refueling() {
            let problem = Problem {
                starts_at: 0,
                day_seconds: vec![60],
                day_steps: vec![4],
                map: Map {
                    width: 3,
                    height: 1,
                    cells: vec![vec![0; 3]],
                },
                spots: vec![tester::model::Spot {
                    brand: usize::MAX,
                    pos: 1,
                    stocks: 1,
                }],
                agents: vec![0, 2],
                fuel_limits: 2,
                players: 1,
                busy_threshold: 1,
                jammed_threshold: 2,
            };
            let mut g = Game::new(problem, &[0, 1]);
            g.agents[0].pos = 1;
            g.agents[1].pos = 1;
            g.agents[0].fuel = 0;
            let ctx = Context::new(&g, 0);
            let stop = Stop {
                point: 0,
                mode: 0,
                meeting: 1,
            };
            let tour = Tour(vec![vec![stop], vec![stop]]);
            let plan = ctx.materialize(&tour);
            assert_eq!(plan, vec![vec![-1, -3]; 2]);
            g.play_day(plan).unwrap();
            assert_eq!(g.agents[0].fuel, 2);
            assert_eq!(g.score.unique_brands, 1);
        }

        #[test]
        fn circular_meeting_dependencies_are_rejected() {
            let mut p: Problem =
                serde_json::from_str(include_str!("../../../official/sample_maps/sample_16.json"))
                    .unwrap();
            p.players = 1;
            let game = Game::new(p, &[0, 0, 0, 1]);
            let ctx = Context::new(&game, 0);
            let first = Stop {
                point: 0,
                mode: 0,
                meeting: 1,
            };
            let second = Stop {
                point: 1,
                mode: 0,
                meeting: 2,
            };
            let tour = Tour(vec![
                vec![first, second],
                vec![],
                vec![],
                vec![second, first],
            ]);
            assert!(ctx.evaluate(&tour, None).is_none());
        }

        #[test]
        fn thirty_second_spot_and_eight_collectors_respect_stock_caps() {
            for stock in [1, 3, 8] {
                for collectors in 0..=8 {
                    let problem = Problem {
                        starts_at: 0,
                        day_seconds: vec![60],
                        day_steps: vec![2],
                        map: Map {
                            width: 9,
                            height: 5,
                            cells: vec![vec![0; 9]; 5],
                        },
                        spots: (0..32)
                            .map(|s| tester::model::Spot {
                                brand: usize::MAX - s * 17,
                                pos: s + 9,
                                stocks: stock,
                            })
                            .collect(),
                        agents: (0..8).collect(),
                        fuel_limits: 20,
                        players: 1,
                        busy_threshold: 1,
                        jammed_threshold: 2,
                    };
                    let kinds: Vec<_> = (0..8).map(|a| u8::from(a >= collectors)).collect();
                    let mut game = Game::new(problem, &kinds);
                    for a in &mut game.agents {
                        a.pos = 40;
                    }
                    let ctx = Context::new(&game, 0);
                    let tour = ctx.empty();
                    let e = ctx.evaluate(&tour, None).unwrap();
                    let plan = ctx.materialize(&tour);
                    game.play_day(plan).unwrap();
                    assert_eq!(e.udon, stock.min(collectors as u32));
                    assert_eq!(e.udon, game.score.udon);
                    assert_eq!(e.daily as usize, game.score.daily_brands);
                    assert_eq!(e.unique as usize, game.score.unique_brands);
                }
            }
        }
    }

    pub use planner::{Memory, plan, select};

    pub struct Rng(u64);
    impl Rng {
        pub fn new(seed: u64) -> Self {
            Self(seed.wrapping_add(0x9e3779b97f4a7c15))
        }
        pub fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        pub fn index(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        pub fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
        }
    }
}

use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};
use tester::engine::Game;
use tester::model::{Request, Response};

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut rng = solver::Rng::new(seed);
    let mut game: Option<Game> = None;
    let mut pending = None;
    let mut memory = solver::Memory::default();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let started = Instant::now();
        let request: Request = serde_json::from_str(&line).expect("valid request");
        let margin = (request.time_limit_ms / 100).clamp(10, 600);
        let budget = request.time_limit_ms.saturating_sub(margin);
        let deadline = started + Duration::from_millis(budget);
        let answer = if let Some(problem) = request.problem {
            let kinds = solver::select(&problem, deadline, &mut rng, &mut memory);
            game = Some(Game::new(problem, &kinds));
            serde_json::to_value(kinds).unwrap()
        } else {
            let g = game.as_mut().expect("selection precedes planning");
            let state = request.state.expect("planning observation");
            if pending.is_none() && g.day == 0 && request.previous_accepted == Some(false) {
                let kinds: Vec<_> = state.agents.iter().map(|a| a.kind).collect();
                *g = Game::new(g.problem.clone(), &kinds);
                memory = solver::Memory::default();
            }
            if let Some(plan) = pending.take() {
                let plan = if request.previous_accepted == Some(false) {
                    g.waiting_plan()
                } else {
                    plan
                };
                let replay = g.play_day(plan).expect("previous plan is valid");
                memory.observe(&replay);
            }
            assert_eq!(g.day, state.day);
            assert_eq!(g.agents, state.agents);
            assert_eq!(g.traffic(), state.traffics);
            let plan = solver::plan(g, deadline, &mut rng, &mut memory);
            let mut checked = g.clone();
            checked
                .play_day(plan.clone())
                .expect("final independent validation");
            pending = Some(plan.clone());
            serde_json::to_value(plan).unwrap()
        };
        serde_json::to_writer(
            &mut stdout,
            &Response {
                request_id: request.request_id,
                answer,
            },
        )
        .unwrap();
        writeln!(stdout).unwrap();
        stdout.flush().unwrap();
        eprintln!(
            "request={} elapsed_ms={}",
            request.request_id,
            started.elapsed().as_millis()
        );
    }
}
