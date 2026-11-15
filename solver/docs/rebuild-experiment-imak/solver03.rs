// solver03: 再構築2。出典コミット: c42d3ec4f177d24b83caed5001922724ba52c158
// アルゴリズム・設定・対応表: solver/docs/rebuild-experiment-imak/README.md
// 詳細: solver/docs/rebuild-experiment-imak/solver03/algorithm.md
// 記録用の単一ファイル。開発用の main.rs・solver/*.rs は別途維持する。
// 通信・探索・テストを収録する。tester と serde_json は Cargo 依存。
// 元の外部モジュールをインライン化し、探索処理・定数・テストを保持している。

// 元ファイル: solver/mod.rs
mod solver {
    // 元ファイル: solver/paths.rs
    mod paths {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        use tester::model::{Observation, Problem};

        pub const MODES: usize = 3;

        #[derive(Clone, Default)]
        pub struct Path {
            pub time: i32,
            pub fuel: i32,
            pub shops: u32,
            pub commands: Vec<i64>,
        }

        pub struct Paths {
            pub points: Vec<usize>,
            pub starts: Vec<usize>,
            pub paths: Vec<Path>,
            pub at_shop: Vec<u32>,
        }

        impl Paths {
            pub fn new(p: &Problem, state: &Observation) -> Self {
                let mut points: Vec<usize> = p.spots.iter().map(|s| s.pos).collect();
                let mut starts = Vec::new();
                for a in &state.agents {
                    let index = points.iter().position(|&q| q == a.pos).unwrap_or_else(|| {
                        points.push(a.pos);
                        points.len() - 1
                    });
                    starts.push(index);
                }
                let mut at_shop = vec![0u32; p.map.len()];
                for (i, s) in p.spots.iter().enumerate() {
                    at_shop[s.pos] = 1 << i;
                }
                let mut times: Vec<i32> = (0..p.map.len())
                    .map(|q| match p.map.terrain(q) {
                        0 => 2,
                        1 => 1,
                        2 => 3,
                        _ => 10000,
                    })
                    .collect();
                let fuels: Vec<i32> = (0..p.map.len())
                    .map(|q| if p.map.terrain(q) == 0 { 1 } else { 2 })
                    .collect();
                for t in &state.traffics {
                    times[t.pos] = [1, 2, 4][t.status as usize];
                }
                let n = points.len();
                let mut paths = vec![Path::default(); MODES * n * n];
                for (i, &source) in points.iter().enumerate() {
                    for mode in 0..MODES {
                        let mut distance = vec![i32::MAX; p.map.len()];
                        let mut previous = vec![(usize::MAX, 0); p.map.len()];
                        let mut queue = BinaryHeap::new();
                        distance[source] = 0;
                        queue.push(Reverse((0, source)));
                        while let Some(Reverse((d, q))) = queue.pop() {
                            if d != distance[q] {
                                continue;
                            }
                            let weight = match mode {
                                0 => times[q] * 1024 + fuels[q],
                                1 => times[q] * 16 + fuels[q] * 12,
                                _ => fuels[q] * 1024 + times[q],
                            };
                            for dir in 0..6 {
                                if let Some(next) = p.map.neighbor(q, dir) {
                                    if p.map.terrain(next) == 3 {
                                        continue;
                                    }
                                    if d + weight < distance[next] {
                                        distance[next] = d + weight;
                                        previous[next] = (q, dir as i64);
                                        queue.push(Reverse((d + weight, next)));
                                    }
                                }
                            }
                        }
                        for (j, &destination) in points.iter().enumerate() {
                            let path = &mut paths[(i * n + j) * MODES + mode];
                            path.shops = at_shop[destination];
                            let mut q = destination;
                            while q != source {
                                let (parent, dir) = previous[q];
                                assert_ne!(parent, usize::MAX, "disconnected map");
                                path.time += times[parent];
                                path.fuel += fuels[parent];
                                path.shops |= at_shop[parent];
                                path.commands.push(dir);
                                q = parent;
                            }
                            path.commands.reverse();
                        }
                    }
                }
                Self {
                    points,
                    starts,
                    paths,
                    at_shop,
                }
            }

            #[inline]
            pub fn get(&self, from: usize, to: usize, mode: u8) -> &Path {
                &self.paths[(from * self.points.len() + to) * MODES + mode as usize]
            }
        }
    }

    #[cfg(test)]
    // 元ファイル: solver/tests.rs
    mod tests {
        use super::*;
        use tester::model::{Map, Spot};

        fn small_problem(steps: Vec<u32>) -> Problem {
            Problem {
                starts_at: 0,
                day_seconds: vec![60; steps.len()],
                day_steps: steps,
                map: Map {
                    width: 6,
                    height: 1,
                    cells: vec![vec![0; 6]],
                },
                spots: vec![
                    Spot {
                        brand: 19,
                        pos: 2,
                        stocks: 2,
                    },
                    Spot {
                        brand: 83,
                        pos: 5,
                        stocks: 1,
                    },
                ],
                agents: vec![0, 1],
                fuel_limits: 3,
                players: 1,
                busy_threshold: 1,
                jammed_threshold: 3,
            }
        }

        fn assert_prediction(context: &Context<'_>, game: &Game, result: &Evaluation) -> Game {
            assert!(result.feasible);
            let mut after = game.clone();
            after
                .play_day(result.plan.clone())
                .expect("the independent simulator must accept every feasible plan");
            assert_eq!(
                after.score.daily_brands - game.score.daily_brands,
                result.brands.count_ones() as usize
            );
            assert_eq!(after.score.udon - game.score.udon, result.udon);
            for (i, a) in after.agents.iter().enumerate() {
                assert_eq!(a.pos, context.paths.points[result.end_points[i]]);
                assert!(
                    a.fuel as i32 >= result.end_fuel[i],
                    "incidental refills may only increase fuel"
                );
            }
            after
        }

        #[test]
        fn meeting_synchronizes_arrival_and_refills_before_the_next_leg() {
            let game = Game::new(small_problem(vec![10]), &[0, 1]);
            let context = Context::new(&game, 0);
            let mut schedule = Schedule::new(2);
            schedule.meetings[0] = Meeting {
                car: 0,
                truck: 1,
                point: 0,
                active: true,
            };
            schedule.routes[0] = vec![
                Stop {
                    target: 0,
                    meeting: Some(0),
                    mode: 0,
                },
                Stop {
                    target: 1,
                    meeting: None,
                    mode: 0,
                },
            ];
            schedule.routes[1] = vec![Stop {
                target: 0,
                meeting: Some(0),
                mode: 0,
            }];
            let result = context.evaluate(&schedule, true);
            let after = assert_prediction(&context, &game, &result);
            assert_eq!(after.agents[0].fuel, 0);
            assert_eq!(after.score.daily_brands, 2);
            assert_eq!(after.score.udon, 2);
        }

        #[test]
        fn cyclic_meetings_and_insufficient_fuel_are_rejected() {
            let game = Game::new(small_problem(vec![20]), &[0, 1]);
            let context = Context::new(&game, 0);
            let mut schedule = Schedule::new(2);
            schedule.routes[0] = vec![Stop {
                target: 1,
                meeting: None,
                mode: 0,
            }];
            assert!(!context.evaluate(&schedule, true).feasible);
            for id in 0..2 {
                schedule.meetings[id] = Meeting {
                    car: 0,
                    truck: 1,
                    point: id,
                    active: true,
                };
            }
            let stops: Vec<_> = (0..2)
                .map(|id| Stop {
                    target: id,
                    meeting: Some(id),
                    mode: 0,
                })
                .collect();
            schedule.routes[0] = stops.clone();
            schedule.routes[1] = stops.into_iter().rev().collect();
            assert!(!context.evaluate(&schedule, true).feasible);
        }

        #[test]
        fn same_cell_at_time_zero_requires_a_tick_to_refill() {
            let mut game = Game::new(small_problem(vec![8]), &[0, 1]);
            game.agents[0].pos = 2;
            game.agents[1].pos = 2;
            game.agents[0].fuel = 0;
            let context = Context::new(&game, 0);
            let mut schedule = Schedule::new(2);
            schedule.meetings[0] = Meeting {
                car: 0,
                truck: 1,
                point: 0,
                active: true,
            };
            schedule.routes[0] = vec![
                Stop {
                    target: 0,
                    meeting: Some(0),
                    mode: 0,
                },
                Stop {
                    target: 1,
                    meeting: None,
                    mode: 0,
                },
            ];
            schedule.routes[1] = vec![Stop {
                target: 0,
                meeting: Some(0),
                mode: 0,
            }];
            let result = context.evaluate(&schedule, true);
            assert_eq!(result.plan[0][0], -1);
            assert_prediction(&context, &game, &result);
        }

        #[test]
        fn randomized_search_matches_simulator_across_maps_and_days() {
            for seed in 0..12 {
                let problem = tester::generator::generate(seed, None).unwrap().problem;
                let mut kinds = vec![0; problem.agents.len()];
                kinds[seed as usize % problem.agents.len()] = 1;
                let mut game = Game::new(problem, &kinds);
                let mut random = Random::new(seed);
                for _ in 0..3 {
                    let context = Context::new(&game, 0);
                    let result = search(
                        &context,
                        Instant::now() + Duration::from_millis(15),
                        &mut random,
                        None,
                    );
                    game = assert_prediction(&context, &game, &result.evaluation);
                }
            }
        }
    }

    use paths::Paths;
    use std::time::{Duration, Instant};
    use tester::engine::Game;
    use tester::model::{Observation, Plan, Problem};

    pub struct Random(u64);
    impl Random {
        pub fn new(seed: u64) -> Self {
            Self(seed.wrapping_add(0x9e3779b97f4a7c15))
        }
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn index(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 * (1.0 / ((1u64 << 53) as f64))
        }
    }

    const MAX_MEETINGS: usize = 32;

    #[derive(Clone, Copy, Debug)]
    struct Stop {
        target: usize,
        meeting: Option<usize>,
        mode: u8,
    }

    #[derive(Clone, Copy, Debug, Default)]
    struct Meeting {
        car: usize,
        truck: usize,
        point: usize,
        active: bool,
    }

    #[derive(Clone, Debug)]
    struct Schedule {
        routes: Vec<Vec<Stop>>,
        meetings: [Meeting; MAX_MEETINGS],
    }
    impl Schedule {
        fn new(n: usize) -> Self {
            Self {
                routes: vec![Vec::new(); n],
                meetings: [Meeting::default(); MAX_MEETINGS],
            }
        }
        fn remove_meeting(&mut self, id: usize) {
            let m = self.meetings[id];
            self.routes[m.car].retain(|s| s.meeting != Some(id));
            self.routes[m.truck].retain(|s| s.meeting != Some(id));
            self.meetings[id].active = false;
        }
    }

    #[derive(Clone)]
    struct Evaluation {
        value: f64,
        feasible: bool,
        brands: u32,
        udon: u32,
        end_fuel: Vec<i32>,
        end_points: Vec<usize>,
        plan: Plan,
    }

    struct Context<'a> {
        problem: &'a Problem,
        state: Observation,
        paths: Paths,
        cars: Vec<usize>,
        trucks: Vec<usize>,
        brand_masks: Vec<u32>,
        collected_brands: u32,
        future: bool,
    }

    impl<'a> Context<'a> {
        fn new(game: &Game, collected: u32) -> Context<'_> {
            let p = &game.problem;
            let state = game.observation(0);
            let paths = Paths::new(p, &state);
            let mut brands = Vec::new();
            let brand_masks: Vec<u32> = p
                .spots
                .iter()
                .map(|s| {
                    let index = brands
                        .iter()
                        .position(|&b| b == s.brand)
                        .unwrap_or_else(|| {
                            brands.push(s.brand);
                            brands.len() - 1
                        });
                    1 << index
                })
                .collect();
            let collected_brands = brand_masks
                .iter()
                .enumerate()
                .filter(|(i, _)| collected & (1 << i) != 0)
                .fold(0, |mask, (_, &b)| mask | b);
            let cars = state
                .agents
                .iter()
                .enumerate()
                .filter_map(|(i, a)| (a.kind == 0).then_some(i))
                .collect();
            let trucks = state
                .agents
                .iter()
                .enumerate()
                .filter_map(|(i, a)| (a.kind == 1).then_some(i))
                .collect();
            Context {
                problem: p,
                state,
                paths,
                cars,
                trucks,
                brand_masks,
                collected_brands,
                future: game.day + 1 < p.day_steps.len(),
            }
        }

        fn evaluate(&self, schedule: &Schedule, output: bool) -> Evaluation {
            let n = self.state.agents.len();
            let limit = self.problem.day_steps[self.state.day] as i32;
            let capacity = self.problem.fuel_limits as i32;
            let mut times = vec![0i32; n];
            let mut fuel: Vec<i32> = self.state.agents.iter().map(|a| a.fuel as i32).collect();
            let mut positions = self.paths.starts.clone();
            let mut cursors = vec![0; n];
            let mut waiting = vec![None; n];
            let mut seen: Vec<u32> = self
                .state
                .agents
                .iter()
                .map(|a| self.paths.at_shop[a.pos])
                .collect();
            let mut plan = vec![Vec::new(); n];
            let mut shortage = 0;
            let mut unfinished = schedule.routes.iter().map(Vec::len).sum::<usize>();
            while unfinished > 0 {
                let before = unfinished;
                for a in 0..n {
                    while waiting[a].is_none() && cursors[a] < schedule.routes[a].len() {
                        let stop = schedule.routes[a][cursors[a]];
                        let target = stop
                            .meeting
                            .map_or(stop.target, |m| schedule.meetings[m].point);
                        let path = self.paths.get(positions[a], target, stop.mode);
                        times[a] += path.time;
                        if self.state.agents[a].kind == 0 {
                            fuel[a] -= path.fuel;
                            seen[a] |= path.shops;
                        }
                        if output {
                            plan[a].extend_from_slice(&path.commands);
                        }
                        positions[a] = target;
                        cursors[a] += 1;
                        unfinished -= 1;
                        waiting[a] = stop.meeting;
                    }
                }
                for (id, m) in schedule
                    .meetings
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.active)
                {
                    if waiting[m.car] == Some(id) && waiting[m.truck] == Some(id) {
                        let t = times[m.car].max(times[m.truck]).max(1);
                        for a in [m.car, m.truck] {
                            if output && times[a] < t {
                                plan[a].push(-(t - times[a]) as i64);
                            }
                            times[a] = t;
                            waiting[a] = None;
                        }
                        shortage += (-fuel[m.car]).max(0);
                        fuel[m.car] = capacity;
                    }
                }
                if before == unfinished {
                    return Evaluation {
                        value: -1e12,
                        feasible: false,
                        brands: 0,
                        udon: 0,
                        end_fuel: fuel,
                        end_points: positions,
                        plan,
                    };
                }
            }
            let overtime: i32 = times.iter().map(|&t| (t - limit).max(0)).sum();
            for &a in &self.cars {
                shortage += (-fuel[a]).max(0);
            }
            let mut counts = [0u32; 32];
            for &a in &self.cars {
                let mut mask = seen[a];
                while mask != 0 {
                    let i = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    counts[i] += 1;
                }
            }
            let mut brands = 0;
            let mut udon = 0;
            for (i, s) in self.problem.spots.iter().enumerate() {
                if counts[i] > 0 {
                    brands |= self.brand_masks[i];
                    udon += counts[i].min(s.stocks);
                }
            }
            let mut value = brands.count_ones() as f64 * 1000.0
                + (brands & !self.collected_brands).count_ones() as f64 * 2000.0
                + udon as f64 * 5.0;
            value -= overtime as f64 * 110.0 + shortage as f64 * 120.0;
            for &a in &self.cars {
                value -= times[a] as f64 * 0.025;
                if self.future {
                    let nearest = self
                        .trucks
                        .iter()
                        .map(|&b| self.paths.get(positions[a], positions[b], 0).time)
                        .min()
                        .unwrap_or(200);
                    let reserve = fuel[a].max(0).min(capacity / 2) as f64;
                    value += 0.16 * reserve;
                    value -= (nearest - fuel[a].max(0)).max(0) as f64 * 0.6;
                    if positions[a] < self.problem.spots.len() {
                        value += 1.0;
                    }
                }
            }
            if output && overtime == 0 {
                for a in 0..n {
                    if times[a] < limit {
                        plan[a].push(-(limit - times[a]) as i64);
                    }
                }
            }
            Evaluation {
                value,
                feasible: shortage == 0 && overtime == 0,
                brands,
                udon,
                end_fuel: fuel,
                end_points: positions,
                plan,
            }
        }

        fn mutate(&self, schedule: &mut Schedule, random: &mut Random) {
            let car = self.cars[random.index(self.cars.len())];
            let route = &schedule.routes[car];
            match random.index(20) {
                0..=3 => {
                    if route.len() >= 55 {
                        return;
                    }
                    let target = random.index(self.problem.spots.len());
                    if route
                        .iter()
                        .any(|s| s.meeting.is_none() && s.target == target)
                    {
                        return;
                    }
                    let index = random.index(route.len() + 1);
                    schedule.routes[car].insert(
                        index,
                        Stop {
                            target,
                            meeting: None,
                            mode: 0,
                        },
                    );
                }
                4..=5 => {
                    if route.is_empty() {
                        return;
                    }
                    let i = random.index(route.len());
                    if let Some(id) = route[i].meeting {
                        schedule.remove_meeting(id);
                    } else {
                        schedule.routes[car].remove(i);
                    }
                }
                6..=7 => {
                    if route.len() < 2 {
                        return;
                    }
                    let i = random.index(route.len());
                    let j = random.index(route.len());
                    if random.index(2) == 0 {
                        schedule.routes[car].swap(i, j);
                    } else {
                        schedule.routes[car][i.min(j)..=i.max(j)].reverse();
                    }
                }
                8 => {
                    if route.is_empty() {
                        return;
                    }
                    let i = random.index(route.len());
                    if route[i].meeting.is_some() {
                        return;
                    }
                    let other = self.cars[random.index(self.cars.len())];
                    let stop = schedule.routes[car].remove(i);
                    let j = random.index(schedule.routes[other].len() + 1);
                    schedule.routes[other].insert(j, stop);
                }
                9 => {
                    let a = random.index(schedule.routes.len());
                    if schedule.routes[a].is_empty() {
                        return;
                    }
                    let i = random.index(schedule.routes[a].len());
                    schedule.routes[a][i].mode = random.index(paths::MODES) as u8;
                }
                10..=11 => {
                    if self.trucks.is_empty() {
                        return;
                    }
                    let Some(id) = schedule.meetings.iter().position(|m| !m.active) else {
                        return;
                    };
                    if schedule.routes[car]
                        .iter()
                        .filter(|s| s.meeting.is_some())
                        .count()
                        >= 5
                    {
                        return;
                    }
                    let truck = self.trucks[random.index(self.trucks.len())];
                    let i = random.index(schedule.routes[car].len() + 1);
                    let point = if random.index(3) != 0 && !schedule.routes[car].is_empty() {
                        let stop = schedule.routes[car][i.min(schedule.routes[car].len() - 1)];
                        stop.meeting
                            .map_or(stop.target, |m| schedule.meetings[m].point)
                    } else {
                        random.index(self.paths.points.len())
                    };
                    let j = random.index(schedule.routes[truck].len() + 1);
                    schedule.meetings[id] = Meeting {
                        car,
                        truck,
                        point,
                        active: true,
                    };
                    let stop = Stop {
                        target: point,
                        meeting: Some(id),
                        mode: 0,
                    };
                    schedule.routes[car].insert(i, stop);
                    schedule.routes[truck].insert(j, stop);
                }
                12..=14 => {
                    let ids: Vec<usize> = schedule
                        .meetings
                        .iter()
                        .enumerate()
                        .filter_map(|(id, m)| m.active.then_some(id))
                        .collect();
                    if ids.is_empty() {
                        return;
                    }
                    let id = ids[random.index(ids.len())];
                    let m = schedule.meetings[id];
                    match random.index(4) {
                        0 => schedule.meetings[id].point = random.index(self.paths.points.len()),
                        1 => schedule.remove_meeting(id),
                        _ => {
                            let a = if random.index(2) == 0 { m.car } else { m.truck };
                            let i = schedule.routes[a]
                                .iter()
                                .position(|s| s.meeting == Some(id))
                                .unwrap();
                            let stop = schedule.routes[a].remove(i);
                            let j = random.index(schedule.routes[a].len() + 1);
                            schedule.routes[a].insert(j, stop);
                        }
                    }
                }
                15 => {
                    if route.is_empty() {
                        return;
                    }
                    let i = random.index(route.len());
                    if route[i].meeting.is_none() {
                        schedule.routes[car][i].target = random.index(self.problem.spots.len());
                    }
                }
                16..=17 => {
                    let other = self.cars[random.index(self.cars.len())];
                    if car == other || route.is_empty() || schedule.routes[other].is_empty() {
                        return;
                    }
                    let i = random.index(route.len());
                    let j = random.index(schedule.routes[other].len());
                    if route[i].meeting.is_none() && schedule.routes[other][j].meeting.is_none() {
                        let stop = schedule.routes[car][i];
                        schedule.routes[car][i] = schedule.routes[other][j];
                        schedule.routes[other][j] = stop;
                    }
                }
                18 => {
                    if route.len() < 2 {
                        return;
                    }
                    let i = random.index(route.len());
                    let stop = schedule.routes[car].remove(i);
                    let j = random.index(schedule.routes[car].len() + 1);
                    schedule.routes[car].insert(j, stop);
                }
                _ => {
                    let other = self.cars[random.index(self.cars.len())];
                    if car == other {
                        return;
                    }
                    let i = random.index(route.len() + 1);
                    let j = random.index(schedule.routes[other].len() + 1);
                    let a_tail = schedule.routes[car].split_off(i);
                    let b_tail = schedule.routes[other].split_off(j);
                    for stop in &a_tail {
                        if let Some(id) = stop.meeting {
                            schedule.meetings[id].car = other;
                        }
                    }
                    for stop in &b_tail {
                        if let Some(id) = stop.meeting {
                            schedule.meetings[id].car = car;
                        }
                    }
                    schedule.routes[car].extend(b_tail);
                    schedule.routes[other].extend(a_tail);
                }
            }
        }
    }

    struct SearchResult {
        schedule: Schedule,
        evaluation: Evaluation,
        iterations: u64,
    }

    fn search(
        context: &Context<'_>,
        deadline: Instant,
        random: &mut Random,
        initial: Option<&Schedule>,
    ) -> SearchResult {
        let start = Instant::now();
        let duration = deadline
            .saturating_duration_since(start)
            .as_secs_f64()
            .max(0.001);
        let mut current = initial
            .cloned()
            .unwrap_or_else(|| Schedule::new(context.state.agents.len()));
        let mut evaluation = context.evaluate(&current, false);
        let mut best = Schedule::new(context.state.agents.len());
        let mut best_eval = context.evaluate(&best, false);
        if evaluation.feasible && evaluation.value > best_eval.value {
            best = current.clone();
            best_eval = evaluation.clone();
        }
        let mut iterations = 0;
        let mut temperature = 400.0;
        while Instant::now() < deadline {
            for _ in 0..256 {
                let mut next = current.clone();
                context.mutate(&mut next, random);
                let next_eval = context.evaluate(&next, false);
                let gain = next_eval.value - evaluation.value;
                if gain >= 0.0 || random.unit() < (gain / temperature).exp() {
                    current = next;
                    evaluation = next_eval;
                    if evaluation.feasible && evaluation.value > best_eval.value + 1e-9 {
                        best = current.clone();
                        best_eval = evaluation.clone();
                    }
                }
                iterations += 1;
            }
            let progress = start.elapsed().as_secs_f64() / duration;
            let cycle = (progress * 8.0).fract();
            temperature = 800.0f64.powf(1.0 - cycle) * 0.5;
            if iterations % 16384 == 0 && random.index(3) == 0 {
                current = best.clone();
                evaluation = best_eval.clone();
            }
        }
        best_eval = context.evaluate(&best, true);
        SearchResult {
            schedule: best,
            evaluation: best_eval,
            iterations,
        }
    }

    #[derive(Default)]
    pub struct Memory {
        next: Option<Cached>,
    }

    #[derive(Clone)]
    struct Cached {
        state: Observation,
        schedule: Schedule,
    }

    impl Cached {
        fn matches(&self, context: &Context<'_>) -> bool {
            self.state.day == context.state.day
                && self.state.agents == context.state.agents
                && self.state.traffics == context.state.traffics
        }
    }

    fn collect(replay: &tester::model::DayReplay, collected: &mut u32) {
        for f in &replay.frames {
            for event in &f.events {
                if let tester::model::Event::Collect { spot, .. } = event {
                    *collected |= 1 << spot;
                }
            }
        }
    }

    fn score(game: &Game) -> f64 {
        game.score.unique_brands as f64 * 1000000.0
            + game.score.daily_brands as f64 * 1000.0
            + game.score.udon as f64
    }

    struct Candidate {
        schedule: Schedule,
        evaluation: Evaluation,
        next: Option<Cached>,
        future_score: f64,
    }

    fn lookahead(
        game: &Game,
        collected: u32,
        candidate: &mut Candidate,
        budget: Duration,
        deadline: Instant,
        random: &mut Random,
    ) {
        let mut future = game.clone();
        let mut collected = collected;
        let Ok(replay) = future.play_day(candidate.evaluation.plan.clone()) else {
            candidate.future_score = -1e12;
            return;
        };
        collect(&replay, &mut collected);
        let depth = (game.day + 3).min(game.problem.day_steps.len());
        let mut next = None;
        while future.day < depth && Instant::now() < deadline {
            let context = Context::new(&future, collected);
            let initial = if future.day == game.day + 1 {
                candidate
                    .next
                    .as_ref()
                    .filter(|c| c.matches(&context))
                    .map(|c| &c.schedule)
            } else {
                None
            };
            let result = search(
                &context,
                (Instant::now() + budget).min(deadline),
                random,
                initial,
            );
            if next.is_none() {
                next = Some(Cached {
                    state: context.state.clone(),
                    schedule: result.schedule,
                });
            }
            let Ok(replay) = future.play_day(result.evaluation.plan) else {
                return;
            };
            collect(&replay, &mut collected);
        }
        if future.day != depth {
            return;
        }
        let value = score(&future);
        if value > candidate.future_score {
            candidate.future_score = value;
            candidate.next = next;
        }
    }

    pub fn plan(
        game: &Game,
        collected: u32,
        deadline: Instant,
        random: &mut Random,
        memory: &mut Memory,
    ) -> Plan {
        let started = Instant::now();
        let context = Context::new(game, collected);
        let warm = memory.next.take().filter(|c| c.matches(&context));
        let initial = warm.as_ref().map(|c| &c.schedule);
        let available = deadline.saturating_duration_since(Instant::now());
        let horizon_enabled = std::env::var("SOLVER_HORIZON").map_or(true, |v| v != "0");
        if !context.future || !horizon_enabled || available.as_millis() < 100 {
            let result = search(&context, deadline, random, initial);
            eprintln!(
                "day={} brands={} udon={} iterations={} fuel={:?} endpoints={:?}",
                game.day + 1,
                result.evaluation.brands.count_ones(),
                result.evaluation.udon,
                result.iterations,
                result.evaluation.end_fuel,
                result.evaluation.end_points
            );
            return result.evaluation.plan;
        }
        let mut candidates = Vec::new();
        if let Some(warm) = warm {
            let evaluation = context.evaluate(&warm.schedule, true);
            if evaluation.feasible {
                candidates.push(Candidate {
                    schedule: warm.schedule,
                    evaluation,
                    next: None,
                    future_score: -1e12,
                });
            }
        }
        let count = 6;
        let current_budget = available.mul_f64(0.38 / count as f64);
        let mut iterations = 0;
        for i in 0..count {
            if Instant::now() >= deadline {
                break;
            }
            let initial = if i % 2 == 0 {
                candidates
                    .iter()
                    .max_by(|a, b| a.evaluation.value.total_cmp(&b.evaluation.value))
                    .map(|c| &c.schedule)
            } else {
                None
            };
            let result = search(
                &context,
                (Instant::now() + current_budget).min(deadline),
                random,
                initial,
            );
            iterations += result.iterations;
            candidates.push(Candidate {
                schedule: result.schedule,
                evaluation: result.evaluation,
                next: None,
                future_score: -1e12,
            });
        }
        let depth = (game.problem.day_steps.len() - game.day - 1).min(2);
        let future_budget = available.mul_f64(0.29 / candidates.len() as f64 / depth as f64);
        for candidate in &mut candidates {
            lookahead(game, collected, candidate, future_budget, deadline, random);
        }
        candidates.sort_by(|a, b| {
            b.future_score
                .total_cmp(&a.future_score)
                .then(b.evaluation.value.total_cmp(&a.evaluation.value))
        });
        candidates.truncate(3);
        let mut i = 0;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.as_millis() < 10 {
                break;
            }
            let budget = future_budget.mul_f64(2.0).min(remaining / depth as u32);
            let index = i % candidates.len();
            lookahead(
                game,
                collected,
                &mut candidates[index],
                budget,
                deadline,
                random,
            );
            i += 1;
        }
        candidates.sort_by(|a, b| {
            b.future_score
                .total_cmp(&a.future_score)
                .then(b.evaluation.value.total_cmp(&a.evaluation.value))
        });
        let winner = candidates.remove(0);
        memory.next = winner.next;
        eprintln!(
            "day={} brands={} udon={} horizon_score={:.0} iterations={} elapsed_ms={}",
            game.day + 1,
            winner.evaluation.brands.count_ones(),
            winner.evaluation.udon,
            winner.future_score,
            iterations,
            started.elapsed().as_millis()
        );
        winner.evaluation.plan
    }

    fn rollout(
        problem: &Problem,
        kinds: &[u8],
        budget: Duration,
        deadline: Instant,
        random: &mut Random,
    ) -> (f64, Option<Cached>) {
        let mut game = Game::new(problem.clone(), kinds);
        let mut collected = 0u32;
        let mut opening = None;
        while game.day < problem.day_steps.len() && Instant::now() < deadline {
            let context = Context::new(&game, collected);
            let result = search(
                &context,
                (Instant::now() + budget).min(deadline),
                random,
                None,
            );
            if opening.is_none() {
                opening = Some(Cached {
                    state: context.state.clone(),
                    schedule: result.schedule,
                });
            }
            if let Ok(replay) = game.play_day(result.evaluation.plan) {
                collect(&replay, &mut collected);
            } else {
                return (-1e9, None);
            }
        }
        if game.day < problem.day_steps.len() {
            return (-1e9, None);
        }
        (score(&game), opening)
    }

    pub fn select(
        problem: &Problem,
        deadline: Instant,
        random: &mut Random,
        memory: &mut Memory,
    ) -> Vec<u8> {
        let n = problem.agents.len();
        if n == 1 {
            return vec![0];
        }
        let started = Instant::now();
        let available = deadline.saturating_duration_since(started);
        let mut candidates: Vec<(Vec<u8>, f64)> = Vec::new();
        for mask in 1u32..(1 << n) {
            if mask.count_ones() > 2 || mask.count_ones() as usize > n / 2 {
                continue;
            }
            let kinds = (0..n).map(|a| ((mask >> a) & 1) as u8).collect();
            candidates.push((kinds, 0.0));
        }
        let screening = Duration::from_secs_f64(
            (available.as_secs_f64() * 0.48
                / candidates.len() as f64
                / problem.day_steps.len() as f64)
                .max(0.001),
        );
        let mut best = candidates[0].0.clone();
        let mut best_score = -1e15;
        for (kinds, score) in &mut candidates {
            if Instant::now() >= deadline {
                break;
            }
            let (value, opening) = rollout(problem, kinds, screening, deadline, random);
            *score = value;
            if *score > best_score {
                best_score = *score;
                best = kinds.clone();
                memory.next = opening;
            }
        }
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
        candidates.truncate(3);
        let mut iteration = 0;
        while Instant::now() < deadline {
            let kinds = &candidates[iteration % candidates.len()].0;
            let remaining = deadline.saturating_duration_since(Instant::now());
            let budget = screening
                .mul_f64(4.0)
                .min(remaining / problem.day_steps.len() as u32);
            if budget.as_millis() < 2 {
                break;
            }
            let (score, opening) = rollout(problem, kinds, budget, deadline, random);
            if score > best_score {
                best_score = score;
                best = kinds.clone();
                memory.next = opening;
            }
            iteration += 1;
        }
        eprintln!(
            "selection kinds={best:?} estimate={best_score:.0} elapsed_ms={}",
            started.elapsed().as_millis()
        );
        best
    }
}

use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};
use tester::engine::Game;
use tester::model::{Request, Response};

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut random = solver::Random::new(seed);
    let mut game: Option<Game> = None;
    let mut collected = 0u32;
    let mut pending = None;
    let mut memory = solver::Memory::default();
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let received = Instant::now();
        let request: Request = serde_json::from_str(&line).expect("request JSON");
        let safety = (request.time_limit_ms / 40).clamp(15, 1000);
        let usable = request.time_limit_ms.saturating_sub(safety);
        let override_ms = std::env::var("SOLVER_BUDGET_MS")
            .ok()
            .and_then(|x| x.parse::<u64>().ok());
        let deadline = received + Duration::from_millis(override_ms.unwrap_or(usable).min(usable));
        let answer = if request.phase == "select" {
            let problem = request.problem.expect("select problem");
            let kinds = solver::select(&problem, deadline, &mut random, &mut memory);
            game = Some(Game::new(problem, &kinds));
            serde_json::to_value(kinds).unwrap()
        } else {
            let state = request.state.expect("plan state");
            let g = game.as_mut().expect("select before plan");
            if let Some(plan) = pending.take() {
                let plan = if request.previous_accepted == Some(false) {
                    g.waiting_plan()
                } else {
                    plan
                };
                let replay = g.play_day(plan).expect("validated previous plan");
                for frame in replay.frames {
                    for event in frame.events {
                        if let tester::model::Event::Collect { spot, .. } = event {
                            collected |= 1 << spot;
                        }
                    }
                }
            }
            assert_eq!(
                g.agents, state.agents,
                "predicted agents differ from observation"
            );
            assert_eq!(
                g.traffic(),
                state.traffics,
                "predicted traffic differs from observation"
            );
            let plan = solver::plan(g, collected, deadline, &mut random, &mut memory);
            let mut validation = g.clone();
            validation
                .play_day(plan.clone())
                .expect("final plan validation");
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
    }
}
