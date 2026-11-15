// solver05: 再構築4。出典コミット: 6fd85e7caf9de55d383f11933b1d2213c9e04f77
// アルゴリズム・設定・対応表: solver/docs/rebuild-experiment-imak/README.md
// 詳細: solver/docs/rebuild-experiment-imak/solver05/algorithm.md
// 記録用の単一ファイル。開発用の main.rs・solver/*.rs は別途維持する。
// 通信・探索・テストを収録する。tester と serde_json は Cargo 依存。
// 元の外部モジュールをインライン化し、探索処理・定数・テストを保持している。

// 元ファイル: solver/mod.rs
mod solver {
    // 元ファイル: solver/network.rs
    pub mod network {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        use tester::engine::Game;

        pub const MODES: usize = 4;
        #[derive(Clone, Copy, Default, Debug)]
        pub struct Leg {
            pub time: i32,
            pub fuel: i32,
            pub visits: u32,
        }
        pub struct Network {
            pub positions: Vec<usize>,
            pub starts: Vec<u8>,
            pub legs: Vec<Leg>,
            pub directions: Vec<Vec<i64>>,
            pub spot_at: Vec<u32>,
        }
        impl Network {
            pub fn new(game: &Game) -> Self {
                let p = &game.problem;
                let mut positions: Vec<_> = p.spots.iter().map(|s| s.pos).collect();
                let starts = game
                    .agents
                    .iter()
                    .map(|a| {
                        if let Some(i) = positions.iter().position(|&pos| pos == a.pos) {
                            i as u8
                        } else {
                            positions.push(a.pos);
                            (positions.len() - 1) as u8
                        }
                    })
                    .collect();
                let cells = p.map.len();
                let mut spot_at = vec![0; cells];
                for (i, s) in p.spots.iter().enumerate() {
                    spot_at[s.pos] = 1 << i;
                }
                let mut time: Vec<i32> = (0..cells)
                    .map(|pos| match p.map.terrain(pos) {
                        0 => 2,
                        1 => 1,
                        _ => 3,
                    })
                    .collect();
                let fuel: Vec<i32> = (0..cells)
                    .map(|pos| if p.map.terrain(pos) == 0 { 1 } else { 2 })
                    .collect();
                for t in game.traffic() {
                    time[t.pos] = [1, 2, 4][t.status as usize];
                }
                add_meeting_positions(p, &mut positions, &time, &fuel);
                let n = positions.len();
                let mut legs = vec![Leg::default(); n * n * MODES];
                let mut directions = vec![Vec::new(); n * n * MODES];
                for (i, &source) in positions.iter().enumerate() {
                    for (mode, (tw, fw)) in [(100, 1), (100, 30), (100, 100), (1, 100)]
                        .into_iter()
                        .enumerate()
                    {
                        let mut distance = vec![i32::MAX; cells];
                        let mut parent = vec![(usize::MAX, 0); cells];
                        let mut heap = BinaryHeap::new();
                        distance[source] = 0;
                        heap.push(Reverse((0, source)));
                        while let Some(Reverse((d, pos))) = heap.pop() {
                            if distance[pos] != d {
                                continue;
                            }
                            let nd = d + tw * time[pos] + fw * fuel[pos];
                            for direction in 0..6 {
                                let Some(next) = p.map.neighbor(pos, direction) else {
                                    continue;
                                };
                                if p.map.terrain(next) == 3 || nd >= distance[next] {
                                    continue;
                                }
                                distance[next] = nd;
                                parent[next] = (pos, direction as i64);
                                heap.push(Reverse((nd, next)));
                            }
                        }
                        for (j, &dest) in positions.iter().enumerate() {
                            let k = (i * n + j) * MODES + mode;
                            let mut cur = dest;
                            let mut leg = Leg {
                                visits: spot_at[source],
                                ..Leg::default()
                            };
                            while cur != source {
                                leg.visits |= spot_at[cur];
                                let (prev, dir) = parent[cur];
                                assert_ne!(prev, usize::MAX, "all endpoints must be connected");
                                leg.time += time[prev];
                                leg.fuel += fuel[prev];
                                directions[k].push(dir);
                                cur = prev;
                            }
                            directions[k].reverse();
                            legs[k] = leg;
                        }
                    }
                }
                Self {
                    positions,
                    starts,
                    legs,
                    directions,
                    spot_at,
                }
            }
            #[inline(always)]
            pub fn index(&self, from: u8, to: u8, mode: u8) -> usize {
                (from as usize * self.positions.len() + to as usize) * MODES + mode as usize
            }
            #[inline(always)]
            pub fn leg(&self, from: u8, to: u8, mode: u8) -> Leg {
                self.legs[self.index(from, to, mode)]
            }
        }

        /// Add a small set of frequently used road junctions as rendezvous candidates.
        /// The visits still use the original shop indices; only paired meetings use these cells.
        fn add_meeting_positions(
            p: &tester::model::Problem,
            positions: &mut Vec<usize>,
            time: &[i32],
            fuel: &[i32],
        ) {
            let cells = p.map.len();
            let mut votes = vec![0u32; cells];
            for spot in &p.spots {
                let source = spot.pos;
                let mut distance = vec![i32::MAX; cells];
                let mut parent = vec![usize::MAX; cells];
                let mut heap = BinaryHeap::new();
                distance[source] = 0;
                heap.push(Reverse((0, source)));
                while let Some(Reverse((d, pos))) = heap.pop() {
                    if distance[pos] != d {
                        continue;
                    }
                    let nd = d + 100 * time[pos] + fuel[pos];
                    for dir in 0..6 {
                        let Some(next) = p.map.neighbor(pos, dir) else {
                            continue;
                        };
                        if p.map.terrain(next) == 3 || nd >= distance[next] {
                            continue;
                        }
                        distance[next] = nd;
                        parent[next] = pos;
                        heap.push(Reverse((nd, next)));
                    }
                }
                for destination in &p.spots {
                    let mut cur = destination.pos;
                    while cur != source {
                        votes[cur] += 1;
                        cur = parent[cur];
                    }
                }
            }
            fn exclude(
                p: &tester::model::Problem,
                source: usize,
                radius: u8,
                blocked: &mut [bool],
            ) {
                let mut distance = vec![u8::MAX; p.map.len()];
                let mut queue = std::collections::VecDeque::from([source]);
                distance[source] = 0;
                while let Some(pos) = queue.pop_front() {
                    blocked[pos] = true;
                    if distance[pos] >= radius {
                        continue;
                    }
                    for dir in 0..6 {
                        if let Some(next) = p.map.neighbor(pos, dir)
                            && p.map.terrain(next) != 3
                            && distance[next] == u8::MAX
                        {
                            distance[next] = distance[pos] + 1;
                            queue.push_back(next);
                        }
                    }
                }
            }
            let mut blocked = vec![false; cells];
            for &pos in positions.iter() {
                exclude(p, pos, 1, &mut blocked);
            }
            for _ in 0..16 {
                let Some(pos) = (0..cells)
                    .filter(|&pos| !blocked[pos] && votes[pos] > 0)
                    .max_by_key(|&pos| votes[pos])
                else {
                    break;
                };
                positions.push(pos);
                exclude(p, pos, 2, &mut blocked);
            }
        }
    }

    // 元ファイル: solver/random.rs
    pub mod random {
        #[derive(Clone)]
        pub struct Random(u64);
        impl Random {
            pub fn new(seed: u64) -> Self {
                Self(seed ^ 0xd1b54a32d192ed03)
            }
            pub fn next(&mut self) -> u64 {
                self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
                let mut z = self.0;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
                z ^ (z >> 31)
            }
            pub fn below(&mut self, n: usize) -> usize {
                ((self.next() as u128 * n as u128) >> 64) as usize
            }
            pub fn unit(&mut self) -> f64 {
                ((self.next() >> 11) as f64 + 0.5) * (1.0 / (1u64 << 53) as f64)
            }
        }
    }

    // 元ファイル: solver/routing.rs
    pub mod routing {
        use super::network::{MODES, Network};
        use super::random::Random;
        use std::time::{Duration, Instant};
        use tester::engine::Game;
        use tester::model::Plan;

        const NO_MEETING: u8 = u8::MAX;
        const MEETINGS: usize = 32;
        const MAX_STOPS: usize = 48;

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Stop {
            pub node: u8,
            pub mode: u8,
            pub meeting: u8,
        }
        impl Stop {
            fn visit(node: usize, mode: usize) -> Self {
                Self {
                    node: node as u8,
                    mode: mode as u8,
                    meeting: NO_MEETING,
                }
            }
        }
        /// Fixed storage makes cloning a candidate one contiguous copy, and avoids
        /// allocating two temporary vectors when exchanging route tails.
        #[derive(Clone, Copy)]
        pub struct Route {
            stops: [Stop; MAX_STOPS],
            count: u8,
        }
        impl Route {
            #[inline(always)]
            pub fn len(&self) -> usize {
                self.count as usize
            }
            #[inline(always)]
            pub fn is_empty(&self) -> bool {
                self.count == 0
            }
            fn empty() -> Self {
                Self {
                    stops: [Stop {
                        node: 0,
                        mode: 0,
                        meeting: NO_MEETING,
                    }; MAX_STOPS],
                    count: 0,
                }
            }
            #[inline(always)]
            fn insert(&mut self, at: usize, stop: Stop) {
                let n = self.count as usize;
                assert!(at <= n && n < MAX_STOPS);
                self.stops.copy_within(at..n, at + 1);
                self.stops[at] = stop;
                self.count += 1;
            }
            #[inline(always)]
            fn remove(&mut self, at: usize) -> Stop {
                let n = self.count as usize;
                assert!(at < n);
                let stop = self.stops[at];
                self.stops.copy_within(at + 1..n, at);
                self.count -= 1;
                stop
            }
            #[inline(always)]
            fn retain(&mut self, mut keep: impl FnMut(&Stop) -> bool) {
                let mut end = 0;
                for i in 0..self.count as usize {
                    let stop = self.stops[i];
                    if keep(&stop) {
                        self.stops[end] = stop;
                        end += 1;
                    }
                }
                self.count = end as u8;
            }
            #[inline(always)]
            fn split_off(&mut self, at: usize) -> Self {
                let n = self.count as usize;
                assert!(at <= n);
                let mut tail = Self::empty();
                tail.stops[..n - at].copy_from_slice(&self.stops[at..n]);
                tail.count = (n - at) as u8;
                self.count = at as u8;
                tail
            }
            #[inline(always)]
            fn extend(&mut self, tail: Self) {
                let n = self.count as usize;
                let k = tail.count as usize;
                assert!(n + k <= MAX_STOPS);
                self.stops[n..n + k].copy_from_slice(&tail.stops[..k]);
                self.count = (n + k) as u8;
            }
        }
        impl From<Vec<Stop>> for Route {
            fn from(stops: Vec<Stop>) -> Self {
                assert!(stops.len() <= MAX_STOPS);
                let mut route = Self::empty();
                route.stops[..stops.len()].copy_from_slice(&stops);
                route.count = stops.len() as u8;
                route
            }
        }
        impl std::ops::Deref for Route {
            type Target = [Stop];
            fn deref(&self) -> &Self::Target {
                &self.stops[..self.count as usize]
            }
        }
        impl std::ops::DerefMut for Route {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.stops[..self.count as usize]
            }
        }
        impl std::fmt::Debug for Route {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_list().entries(self.iter()).finish()
            }
        }
        impl<'a> IntoIterator for &'a Route {
            type Item = &'a Stop;
            type IntoIter = std::slice::Iter<'a, Stop>;
            fn into_iter(self) -> Self::IntoIter {
                self.stops[..self.count as usize].iter()
            }
        }
        impl<'a> IntoIterator for &'a mut Route {
            type Item = &'a mut Stop;
            type IntoIter = std::slice::IterMut<'a, Stop>;
            fn into_iter(self) -> Self::IntoIter {
                self.stops[..self.count as usize].iter_mut()
            }
        }
        #[derive(Clone, Debug)]
        pub struct Itinerary {
            pub routes: Vec<Route>,
        }
        impl Itinerary {
            pub fn empty(n: usize) -> Self {
                Self {
                    routes: vec![Route::empty(); n],
                }
            }
            pub fn remove_meeting(&mut self, id: u8) {
                for route in &mut self.routes {
                    route.retain(|v| v.meeting != id);
                }
            }
        }
        #[derive(Clone, Debug)]
        pub struct Evaluation {
            pub value: f64,
            pub violation: i32,
            pub masks: [u32; 8],
            pub fuel: [i32; 8],
            pub ends: [u8; 8],
            pub times: [i32; 8],
            pub daily: u32,
            pub udon: u32,
        }

        pub struct Router<'a> {
            pub game: &'a Game,
            pub net: Network,
            pub patrols: Vec<usize>,
            pub supplies: Vec<usize>,
            brands: [[u32; 256]; 4],
            stock_masks: [u32; 8],
            known: u32,
            pub fuel_weight: f64,
            end_values: Vec<Vec<f64>>,
            visit_bonus: Vec<[[f64; 256]; 4]>,
        }
        impl<'a> Router<'a> {
            pub fn new(game: &'a Game, known: u32, fuel_weight: f64) -> Self {
                let net = Network::new(game);
                let mut brand_ids = Vec::new();
                let mut spot_brands = Vec::new();
                let mut stock_masks = [0; 8];
                for (i, s) in game.problem.spots.iter().enumerate() {
                    let id = if let Some(id) = brand_ids.iter().position(|&b| b == s.brand) {
                        id
                    } else {
                        brand_ids.push(s.brand);
                        brand_ids.len() - 1
                    };
                    spot_brands.push(1u32 << id);
                    for mask in stock_masks.iter_mut().take(s.stocks as usize) {
                        *mask |= 1 << i;
                    }
                }
                let mut brands = [[0; 256]; 4];
                for (chunk, table) in brands.iter_mut().enumerate() {
                    for bits in 1usize..256 {
                        let bit = bits.trailing_zeros() as usize;
                        table[bits] = table[bits & (bits - 1)]
                            | spot_brands.get(chunk * 8 + bit).copied().unwrap_or(0);
                    }
                }
                let end_values = vec![vec![0.; net.positions.len()]; game.agents.len()];
                Self {
                    game,
                    net,
                    patrols: (0..game.agents.len())
                        .filter(|&a| game.agents[a].kind == 0)
                        .collect(),
                    supplies: (0..game.agents.len())
                        .filter(|&a| game.agents[a].kind == 1)
                        .collect(),
                    brands,
                    stock_masks,
                    known,
                    fuel_weight,
                    end_values,
                    visit_bonus: Vec::new(),
                }
            }
            /// Encourage finishing near a useful first destination on the next day.
            /// The final choice between trajectories still uses simulated scores.
            pub fn guide_next_day(&mut self, targets: &[Option<usize>], weight: f64) {
                for (a, target) in targets.iter().enumerate() {
                    let Some(pos) = target else {
                        continue;
                    };
                    let Some(node) = self.net.positions.iter().position(|p| p == pos) else {
                        continue;
                    };
                    for end in 0..self.net.positions.len() {
                        let time = self.net.leg(end as u8, node as u8, 0).time as f64;
                        self.end_values[a][end] = -weight * time;
                    }
                }
            }
            /// Small temporary rewards explore different allocations of shops to cars.
            /// They are discarded when independently simulated trajectories are compared.
            pub fn perturb_visits(&mut self, rng: &mut Random, scale: f64) {
                if scale == 0. {
                    return;
                }
                self.visit_bonus = vec![[[0.; 256]; 4]; self.game.agents.len()];
                for &a in &self.patrols {
                    let weights: Vec<_> = (0..self.game.problem.spots.len())
                        .map(|_| (rng.unit() * 2. - 1.) * scale)
                        .collect();
                    for chunk in 0..4 {
                        for bits in 1usize..256 {
                            let bit = bits.trailing_zeros() as usize;
                            self.visit_bonus[a][chunk][bits] = self.visit_bonus[a][chunk]
                                [bits & (bits - 1)]
                                + weights.get(chunk * 8 + bit).copied().unwrap_or(0.);
                        }
                    }
                }
            }
            #[inline(always)]
            pub fn brands_of(&self, spots: u32) -> u32 {
                self.brands[0][(spots & 255) as usize]
                    | self.brands[1][((spots >> 8) & 255) as usize]
                    | self.brands[2][((spots >> 16) & 255) as usize]
                    | self.brands[3][(spots >> 24) as usize]
            }
            #[inline(always)]
            pub fn evaluate(
                &self,
                solution: &Itinerary,
                mut schedule: Option<&mut Vec<Vec<i32>>>,
            ) -> Evaluation {
                let n = self.game.agents.len();
                let mut e = Evaluation {
                    value: 0.,
                    violation: 0,
                    masks: [0; 8],
                    fuel: [0; 8],
                    ends: [0; 8],
                    times: [0; 8],
                    daily: 0,
                    udon: 0,
                };
                let mut cursor = [0usize; 8];
                let mut waiting = [NO_MEETING; 8];
                let mut owner = [usize::MAX; MEETINGS];
                let mut total = 0;
                for a in 0..n {
                    e.ends[a] = self.net.starts[a];
                    e.fuel[a] = self.game.agents[a].fuel as i32;
                    if self.game.agents[a].kind == 0 {
                        e.masks[a] = self.net.spot_at[self.game.agents[a].pos];
                    }
                    total += solution.routes[a].len();
                }
                while total > 0 {
                    let mut progress = false;
                    for a in 0..n {
                        if waiting[a] != NO_MEETING {
                            continue;
                        }
                        while cursor[a] < solution.routes[a].len() {
                            let v = solution.routes[a][cursor[a]];
                            if v.node as usize >= self.net.positions.len() {
                                e.violation = 100000;
                                e.value = -1e15;
                                return e;
                            }
                            let leg = self.net.leg(e.ends[a], v.node, v.mode);
                            e.times[a] += leg.time;
                            e.ends[a] = v.node;
                            if self.game.agents[a].kind == 0 {
                                e.violation += (-e.fuel[a] + leg.fuel).max(0) - (-e.fuel[a]).max(0);
                                e.fuel[a] -= leg.fuel;
                                e.masks[a] |= leg.visits;
                            }
                            progress = true;
                            if v.meeting != NO_MEETING {
                                let id = v.meeting as usize;
                                let b = owner[id];
                                if b == usize::MAX {
                                    owner[id] = a;
                                    waiting[a] = v.meeting;
                                    break;
                                }
                                if b == a
                                    || waiting[b] != v.meeting
                                    || e.ends[b] != v.node
                                    || self.game.agents[a].kind == self.game.agents[b].kind
                                {
                                    e.violation = 100000;
                                    e.value = -1e15;
                                    return e;
                                }
                                let time = e.times[a].max(e.times[b]).max(1);
                                e.times[a] = time;
                                e.times[b] = time;
                                let patrol = if self.game.agents[a].kind == 0 { a } else { b };
                                e.fuel[patrol] = self.game.problem.fuel_limits as i32;
                                if let Some(ref mut times) = schedule {
                                    times[a][cursor[a]] = time;
                                    times[b][cursor[b]] = time;
                                }
                                cursor[b] += 1;
                                total -= 1;
                                waiting[b] = NO_MEETING;
                            } else if let Some(ref mut times) = schedule {
                                times[a][cursor[a]] = e.times[a];
                            }
                            cursor[a] += 1;
                            total -= 1;
                        }
                    }
                    if !progress {
                        e.violation = 100000;
                        e.value = -1e15;
                        return e;
                    }
                }
                let steps = self.game.problem.day_steps[self.game.day] as i32;
                e.violation += e.times[..n]
                    .iter()
                    .map(|&t| (t - steps).max(0))
                    .sum::<i32>();
                let mut levels = [0u32; 8];
                for (i, &a) in self.patrols.iter().enumerate() {
                    let mask = e.masks[a];
                    for j in (1..=i).rev() {
                        levels[j] |= levels[j - 1] & mask;
                    }
                    levels[0] |= mask;
                }
                for (level, stock) in levels.iter().zip(self.stock_masks) {
                    e.udon += (level & stock).count_ones();
                }
                let brands = self.brands_of(levels[0]);
                e.daily = brands.count_ones();
                e.value = e.daily as f64 * 100.
                    + (brands & !self.known).count_ones() as f64 * 1000.
                    + e.udon as f64;
                if !self.visit_bonus.is_empty() {
                    for &a in &self.patrols {
                        let bits = e.masks[a];
                        e.value += self.visit_bonus[a][0][(bits & 255) as usize]
                            + self.visit_bonus[a][1][((bits >> 8) & 255) as usize]
                            + self.visit_bonus[a][2][((bits >> 16) & 255) as usize]
                            + self.visit_bonus[a][3][(bits >> 24) as usize];
                    }
                }
                if self.game.day + 1 < self.game.problem.day_steps.len() {
                    let need = self.game.problem.day_steps[self.game.day + 1] as i32;
                    for &a in &self.patrols {
                        let useful_fuel = if self.supplies.is_empty() {
                            e.fuel[a].max(0)
                        } else {
                            e.fuel[a].clamp(0, need)
                        };
                        e.value += self.fuel_weight * useful_fuel as f64;
                        if let Some(nearest) = self
                            .supplies
                            .iter()
                            .map(|&b| self.net.leg(e.ends[a], e.ends[b], 0).time)
                            .min()
                        {
                            let deficit =
                                (need - e.fuel[a]).clamp(0, need) as f64 / need.max(1) as f64;
                            e.value -= self.fuel_weight * 0.4 * nearest as f64 * deficit;
                        }
                    }
                }
                if self.game.day + 1 < self.game.problem.day_steps.len() {
                    for a in 0..n {
                        e.value += self.end_values[a][e.ends[a] as usize];
                    }
                }
                e.value -= e.times[..n].iter().sum::<i32>() as f64 * 0.005;
                e.value -= solution
                    .routes
                    .iter()
                    .map(|route| route.len())
                    .sum::<usize>() as f64
                    * 0.00001;
                e
            }
            pub fn plan(&self, solution: &Itinerary) -> Option<Plan> {
                let mut times: Vec<_> = solution.routes.iter().map(|r| vec![0; r.len()]).collect();
                let e = self.evaluate(solution, Some(&mut times));
                if e.violation != 0 {
                    return None;
                }
                let steps = self.game.problem.day_steps[self.game.day] as i32;
                let mut plan = Vec::new();
                for (a, route) in solution.routes.iter().enumerate() {
                    let mut actions = Vec::new();
                    let mut pos = self.net.starts[a];
                    let mut time = 0;
                    for (i, stop) in route.iter().enumerate() {
                        let idx = self.net.index(pos, stop.node, stop.mode);
                        actions.extend_from_slice(&self.net.directions[idx]);
                        time += self.net.legs[idx].time;
                        if times[a][i] > time {
                            actions.push(-i64::from(times[a][i] - time));
                            time = times[a][i];
                        }
                        pos = stop.node;
                    }
                    if time < steps {
                        actions.push(-i64::from(steps - time));
                    }
                    plan.push(actions);
                }
                Some(plan)
            }
            /// Remove zero-length visits without changing the emitted action plan.
            fn compact(&self, solution: &mut Itinerary) {
                for (a, route) in solution.routes.iter_mut().enumerate() {
                    let mut previous = self.net.starts[a];
                    let mut end = 0;
                    for i in 0..route.count as usize {
                        let mut stop = route.stops[i];
                        if stop.meeting == NO_MEETING && stop.node == previous {
                            continue;
                        }
                        if stop.meeting != NO_MEETING
                            && end > 0
                            && route.stops[end - 1].meeting == NO_MEETING
                            && route.stops[end - 1].node == stop.node
                        {
                            stop.mode = route.stops[end - 1].mode;
                            route.stops[end - 1] = stop;
                        } else {
                            route.stops[end] = stop;
                            end += 1;
                        }
                        previous = stop.node;
                    }
                    route.count = end as u8;
                }
            }
            fn unpair_meeting(&self, solution: &mut Itinerary, id: u8) {
                for (a, route) in solution.routes.iter_mut().enumerate() {
                    if self.game.agents[a].kind == 0 {
                        for stop in route {
                            if stop.meeting == id {
                                stop.meeting = NO_MEETING;
                            }
                        }
                    } else {
                        route.retain(|stop| stop.meeting != id);
                    }
                }
            }
            #[inline(always)]
            fn mutate(&self, s: &mut Itinerary, rng: &mut Random, seen: Option<&Evaluation>) {
                if self.patrols.is_empty() {
                    return;
                }
                let a = self.patrols[rng.below(self.patrols.len())];
                let kind = rng.below(100);
                match kind {
                    0..=23 => {
                        if s.routes[a].len() >= MAX_STOPS {
                            return;
                        }
                        let n = self.game.problem.spots.len();
                        let node = if let Some(e) = seen.filter(|_| rng.below(4) != 0) {
                            let all = u32::MAX >> (32 - n);
                            let mut available = all & !e.masks[a];
                            if available == 0 {
                                return;
                            }
                            for _ in 0..rng.below(available.count_ones() as usize) {
                                available &= available - 1;
                            }
                            available.trailing_zeros() as usize
                        } else {
                            rng.below(n)
                        };
                        let mode = rng.below(MODES);
                        let at = if rng.below(3) == 0 {
                            rng.below(s.routes[a].len() + 1)
                        } else {
                            (0..=s.routes[a].len())
                                .min_by_key(|&i| {
                                    let before = if i == 0 {
                                        self.net.starts[a]
                                    } else {
                                        s.routes[a][i - 1].node
                                    };
                                    let mut delta =
                                        self.net.leg(before, node as u8, mode as u8).time;
                                    if let Some(after) = s.routes[a].get(i) {
                                        delta +=
                                            self.net.leg(node as u8, after.node, after.mode).time
                                                - self.net.leg(before, after.node, after.mode).time;
                                    }
                                    delta
                                })
                                .unwrap()
                        };
                        let before = if at == 0 {
                            self.net.starts[a]
                        } else {
                            s.routes[a][at - 1].node
                        };
                        if before as usize == node
                            || s.routes[a]
                                .get(at)
                                .is_some_and(|stop| stop.node as usize == node)
                        {
                            return;
                        }
                        s.routes[a].insert(at, Stop::visit(node, mode));
                    }
                    24..=34 => {
                        if s.routes[a].is_empty() {
                            return;
                        }
                        let at = rng.below(s.routes[a].len());
                        let v = s.routes[a][at];
                        if v.meeting == NO_MEETING {
                            s.routes[a].remove(at);
                        } else if rng.below(2) == 0 {
                            self.unpair_meeting(s, v.meeting);
                        } else {
                            s.remove_meeting(v.meeting);
                        }
                    }
                    35..=48 => {
                        let b = if rng.below(2) == 0 {
                            a
                        } else {
                            self.patrols[rng.below(self.patrols.len())]
                        };
                        if s.routes[a].is_empty() || s.routes[b].len() >= MAX_STOPS {
                            return;
                        }
                        let at = rng.below(s.routes[a].len());
                        let v = s.routes[a].remove(at);
                        let to = rng.below(s.routes[b].len() + 1);
                        s.routes[b].insert(to, v);
                    }
                    49..=57 => {
                        let b = self.patrols[rng.below(self.patrols.len())];
                        if s.routes[a].is_empty() || s.routes[b].is_empty() {
                            return;
                        }
                        let i = rng.below(s.routes[a].len());
                        let j = rng.below(s.routes[b].len());
                        let v = s.routes[a][i];
                        s.routes[a][i] = s.routes[b][j];
                        s.routes[b][j] = v;
                    }
                    58..=65 => {
                        if s.routes[a].len() < 2 {
                            return;
                        }
                        let i = rng.below(s.routes[a].len());
                        let j = rng.below(s.routes[a].len());
                        s.routes[a][i.min(j)..=i.max(j)].reverse();
                    }
                    66..=70 => {
                        let b = self.patrols[rng.below(self.patrols.len())];
                        if a == b {
                            return;
                        }
                        let i = rng.below(s.routes[a].len() + 1);
                        let j = rng.below(s.routes[b].len() + 1);
                        if i + s.routes[b].len() - j > MAX_STOPS
                            || j + s.routes[a].len() - i > MAX_STOPS
                        {
                            return;
                        }
                        let tail_a = s.routes[a].split_off(i);
                        let tail_b = s.routes[b].split_off(j);
                        s.routes[a].extend(tail_b);
                        s.routes[b].extend(tail_a);
                    }
                    71..=79 => {
                        let a = rng.below(s.routes.len());
                        if s.routes[a].is_empty() {
                            return;
                        }
                        let at = rng.below(s.routes[a].len());
                        if kind < 76 {
                            s.routes[a][at].mode = rng.below(MODES) as u8;
                        } else {
                            let node = rng.below(self.net.positions.len()) as u8;
                            let id = s.routes[a][at].meeting;
                            if id == NO_MEETING {
                                s.routes[a][at].node =
                                    (node as usize % self.game.problem.spots.len()) as u8;
                            } else {
                                for route in &mut s.routes {
                                    for v in route {
                                        if v.meeting == id {
                                            v.node = node;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    80..=92 => {
                        if self.supplies.is_empty() || s.routes[a].len() >= MAX_STOPS {
                            return;
                        }
                        let b = self.supplies[rng.below(self.supplies.len())];
                        if s.routes[b].len() >= MAX_STOPS {
                            return;
                        }
                        let used = s
                            .routes
                            .iter()
                            .flatten()
                            .filter(|v| v.meeting != NO_MEETING)
                            .fold(0u32, |m, v| m | (1 << v.meeting));
                        if used == u32::MAX {
                            return;
                        }
                        let id = (!used).trailing_zeros() as u8;
                        let i = rng.below(s.routes[a].len() + 1);
                        let node = if rng.below(3) > 0 {
                            if i == 0 {
                                self.net.starts[a]
                            } else {
                                s.routes[a][i - 1].node
                            }
                        } else {
                            rng.below(self.net.positions.len()) as u8
                        };
                        let j = if rng.below(3) == 0 {
                            rng.below(s.routes[b].len() + 1)
                        } else {
                            (0..=s.routes[b].len())
                                .min_by_key(|&j| {
                                    let before = if j == 0 {
                                        self.net.starts[b]
                                    } else {
                                        s.routes[b][j - 1].node
                                    };
                                    self.net.leg(before, node, 0).time
                                        + s.routes[b].get(j).map_or(0, |after| {
                                            self.net.leg(node, after.node, 0).time
                                                - self.net.leg(before, after.node, 0).time
                                        })
                                })
                                .unwrap()
                        };
                        s.routes[a].insert(
                            i,
                            Stop {
                                node,
                                mode: rng.below(MODES) as u8,
                                meeting: id,
                            },
                        );
                        s.routes[b].insert(
                            j,
                            Stop {
                                node,
                                mode: 0,
                                meeting: id,
                            },
                        );
                    }
                    _ => {
                        let a = rng.below(s.routes.len());
                        if s.routes[a].is_empty() {
                            return;
                        }
                        let i = rng.below(s.routes[a].len());
                        if kind < 97 {
                            let v = s.routes[a].remove(i);
                            let j = rng.below(s.routes[a].len() + 1);
                            s.routes[a].insert(j, v);
                        } else {
                            let id = s.routes[a][i].meeting;
                            if id != NO_MEETING {
                                s.remove_meeting(id);
                            }
                        }
                    }
                }
            }
            /// Enumerate feasible local improvements after annealing. Every accepted
            /// candidate improves the same objective used by the main search.
            fn polish(&self, initial: &Itinerary, deadline: Instant) -> Itinerary {
                let mut best = initial.clone();
                self.compact(&mut best);
                let mut be = self.evaluate(&best, None);
                let mut candidate = best.clone();
                let mut checks = 0usize;
                loop {
                    let base = best.clone();
                    let old_value = be.value;
                    macro_rules! consider {
                        () => {{
                            checks += 1;
                            if checks.is_multiple_of(128) && Instant::now() >= deadline {
                                return best;
                            }
                            let e = self.evaluate(&candidate, None);
                            if e.violation == 0 && e.value > be.value + 1e-8 {
                                best.clone_from(&candidate);
                                be = e;
                            }
                        }};
                    }
                    for a in 0..base.routes.len() {
                        for i in 0..base.routes[a].len() {
                            let stop = base.routes[a][i];
                            candidate.clone_from(&base);
                            if stop.meeting == NO_MEETING {
                                candidate.routes[a].remove(i);
                            } else {
                                candidate.remove_meeting(stop.meeting);
                            }
                            consider!();
                            if stop.meeting != NO_MEETING {
                                candidate.clone_from(&base);
                                self.unpair_meeting(&mut candidate, stop.meeting);
                                consider!();
                            }
                            for mode in 0..MODES {
                                if mode as u8 == stop.mode {
                                    continue;
                                }
                                candidate.clone_from(&base);
                                candidate.routes[a][i].mode = mode as u8;
                                consider!();
                            }
                            let destinations = if stop.meeting == NO_MEETING {
                                self.game.problem.spots.len()
                            } else {
                                self.net.positions.len()
                            };
                            for node in 0..destinations {
                                if node as u8 == stop.node {
                                    continue;
                                }
                                candidate.clone_from(&base);
                                if stop.meeting == NO_MEETING {
                                    candidate.routes[a][i].node = node as u8;
                                } else {
                                    for route in &mut candidate.routes {
                                        for v in route {
                                            if v.meeting == stop.meeting {
                                                v.node = node as u8;
                                            }
                                        }
                                    }
                                }
                                consider!();
                            }
                            if stop.meeting == NO_MEETING {
                                for &b in &self.patrols {
                                    if base.routes[b].len() >= MAX_STOPS {
                                        continue;
                                    }
                                    for j in 0..=base.routes[b].len() {
                                        if a == b && (j == i || j == i + 1) {
                                            continue;
                                        }
                                        candidate.clone_from(&base);
                                        candidate.routes[a].remove(i);
                                        let at = if a == b && j > i { j - 1 } else { j };
                                        candidate.routes[b].insert(at, stop);
                                        consider!();
                                    }
                                }
                            }
                        }
                    }
                    let base_eval = self.evaluate(&base, None);
                    for &a in &self.patrols {
                        if base.routes[a].len() >= MAX_STOPS {
                            continue;
                        }
                        for node in 0..self.game.problem.spots.len() {
                            if base_eval.masks[a] & (1 << node) != 0 {
                                continue;
                            }
                            for i in 0..=base.routes[a].len() {
                                for mode in 0..MODES {
                                    candidate.clone_from(&base);
                                    candidate.routes[a].insert(i, Stop::visit(node, mode));
                                    consider!();
                                }
                            }
                        }
                        for i in 0..base.routes[a].len() {
                            for j in i + 1..base.routes[a].len() {
                                candidate.clone_from(&base);
                                candidate.routes[a][i..=j].reverse();
                                consider!();
                            }
                        }
                    }
                    if be.value <= old_value + 1e-8 || Instant::now() >= deadline {
                        return best;
                    }
                }
            }
            pub fn improve(
                &self,
                initial: Option<&Itinerary>,
                deadline: Instant,
                rng: &mut Random,
            ) -> Itinerary {
                #[cfg(target_arch = "x86_64")]
                if std::is_x86_feature_detected!("popcnt") {
                    // Runtime detection ensures this code is only executed on supported CPUs.
                    return unsafe { self.improve_popcnt(initial, deadline, rng) };
                }
                self.improve_inner(initial, deadline, rng)
            }
            #[cfg(target_arch = "x86_64")]
            #[target_feature(enable = "popcnt")]
            unsafe fn improve_popcnt(
                &self,
                initial: Option<&Itinerary>,
                deadline: Instant,
                rng: &mut Random,
            ) -> Itinerary {
                self.improve_inner(initial, deadline, rng)
            }
            #[inline(always)]
            fn improve_inner(
                &self,
                initial: Option<&Itinerary>,
                deadline: Instant,
                rng: &mut Random,
            ) -> Itinerary {
                let empty = Itinerary::empty(self.game.agents.len());
                let mut best = initial
                    .filter(|s| self.evaluate(s, None).violation == 0)
                    .unwrap_or(&empty)
                    .clone();
                self.compact(&mut best);
                let mut be = self.evaluate(&best, None);
                let mut current = initial.unwrap_or(&best).clone();
                let mut ce = self.evaluate(&current, None);
                let mut candidate = best.clone();
                let started = Instant::now();
                let budget = deadline.saturating_duration_since(started).as_secs_f64();
                let anneal_deadline = started + Duration::from_secs_f64(budget * 0.9);
                let mut iterations = 0u64;
                let mut temp = 10.;
                let mut penalty = 12.;
                let mut phase = 0u64;
                loop {
                    if iterations.is_multiple_of(512) {
                        let now = Instant::now();
                        if now >= anneal_deadline {
                            break;
                        }
                        let fraction =
                            now.duration_since(started).as_secs_f64() / (budget * 0.9).max(1e-9);
                        let cycle = (fraction * 8.).fract();
                        let next_phase = (fraction * 8.) as u64;
                        if next_phase != phase {
                            current.clone_from(&best);
                            ce = be.clone();
                            phase = next_phase;
                        }
                        temp = (4.0f64 * (0.008f64).powf(cycle)) * (1. - 0.65 * fraction);
                        penalty = 4. + 24. * cycle;
                    }
                    candidate.clone_from(&current);
                    self.mutate(&mut candidate, rng, Some(&ce));
                    let e = self.evaluate(&candidate, None);
                    if e.violation == 0 && e.value > be.value + 1e-8 {
                        best.clone_from(&candidate);
                        self.compact(&mut best);
                        be = self.evaluate(&best, None);
                    }
                    let delta = e.value - ce.value - penalty * (e.violation - ce.violation) as f64;
                    if delta >= 0. || (delta / temp).exp() > rng.unit() {
                        std::mem::swap(&mut current, &mut candidate);
                        ce = e;
                    }
                    iterations += 1;
                }
                best = self.polish(&best, deadline);
                self.compact(&mut best);
                be = self.evaluate(&best, None);
                if budget >= 0.1 {
                    eprintln!(
                        "day={} search_ms={} trials={} daily={} udon={} fuel={:?}",
                        self.game.day,
                        started.elapsed().as_millis(),
                        iterations,
                        be.daily,
                        be.udon,
                        &be.fuel[..self.game.agents.len()]
                    );
                }
                best
            }
            pub fn quick(
                &self,
                initial: Option<&Itinerary>,
                duration: Duration,
                rng: &mut Random,
            ) -> Itinerary {
                self.improve(initial, Instant::now() + duration, rng)
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use tester::generator;
            use tester::model::{Map, Problem, Spot};
            fn tiny() -> Game {
                Game::new(
                    Problem {
                        starts_at: 0,
                        day_seconds: vec![60],
                        day_steps: vec![8],
                        map: Map {
                            width: 4,
                            height: 1,
                            cells: vec![vec![0; 4]],
                        },
                        spots: vec![Spot {
                            brand: 200,
                            pos: 2,
                            stocks: 2,
                        }],
                        agents: vec![0, 0],
                        fuel_limits: 2,
                        players: 1,
                        busy_threshold: 1,
                        jammed_threshold: 3,
                    },
                    &[0, 1],
                )
            }
            #[test]
            fn fixed_routes_match_vec_operations_including_capacity_boundaries() {
                let mut route = Route::empty();
                let mut model = Vec::new();
                let mut rng = Random::new(777);
                for _ in 0..20000 {
                    match rng.below(6) {
                        0 | 1 if model.len() < MAX_STOPS => {
                            let at = rng.below(model.len() + 1);
                            let stop = Stop::visit(rng.below(32), rng.below(MODES));
                            route.insert(at, stop);
                            model.insert(at, stop);
                        }
                        2 if !model.is_empty() => {
                            let at = rng.below(model.len());
                            assert_eq!(route.remove(at), model.remove(at));
                        }
                        3 => {
                            let at = rng.below(model.len() + 1);
                            let tail = route.split_off(at);
                            let other = model.split_off(at);
                            assert_eq!(&*tail, other.as_slice());
                            route.extend(tail);
                            model.extend(other);
                        }
                        4 => {
                            route.reverse();
                            model.reverse();
                        }
                        _ => {
                            route.retain(|s| !s.node.is_multiple_of(7));
                            model.retain(|s| !s.node.is_multiple_of(7));
                        }
                    }
                    assert_eq!(&*route, model.as_slice());
                }
                while model.len() < MAX_STOPS {
                    route.insert(route.len(), Stop::visit(1, 0));
                    model.push(Stop::visit(1, 0));
                }
                assert_eq!(&*route, model.as_slice());
                let tail = route.split_off(0);
                assert!(route.is_empty());
                route.extend(tail);
                assert_eq!(&*route, model.as_slice());
            }
            #[test]
            fn redundant_visits_compact_to_identical_actions() {
                let game = tiny();
                let router = Router::new(&game, 0, 0.);
                let meet = Stop {
                    node: 0,
                    mode: 0,
                    meeting: 0,
                };
                let start = Stop {
                    node: router.net.starts[0],
                    mode: 0,
                    meeting: NO_MEETING,
                };
                let mut solution = Itinerary {
                    routes: vec![
                        vec![
                            start,
                            Stop::visit(0, 1),
                            Stop::visit(0, 3),
                            meet,
                            Stop::visit(0, 2),
                        ]
                        .into(),
                        vec![meet].into(),
                    ],
                };
                let before = router.plan(&solution).unwrap();
                let e = router.evaluate(&solution, None);
                router.compact(&mut solution);
                let after = router.plan(&solution).unwrap();
                assert_eq!(before, after);
                assert_eq!(solution.routes[0].len(), 1);
                assert_eq!(solution.routes[0][0].mode, 1);
                assert!(router.evaluate(&solution, None).value > e.value);
            }
            #[test]
            fn initial_same_cell_rendezvous_waits_for_refill() {
                let mut g = tiny();
                g.agents[0].fuel = 0;
                let r = Router::new(&g, 0, 0.);
                let meeting = Stop {
                    node: r.net.starts[0],
                    mode: 0,
                    meeting: 0,
                };
                let s = Itinerary {
                    routes: vec![
                        vec![meeting, Stop::visit(0, 0)].into(),
                        vec![meeting].into(),
                    ],
                };
                let plan = r.plan(&s).unwrap();
                assert_eq!(plan[0][0], -1);
                let e = r.evaluate(&s, None);
                let replay = g.play_day(plan).unwrap();
                assert_eq!(replay.frames.last().unwrap().score.udon, e.udon);
            }
            #[test]
            fn cyclic_meetings_are_rejected() {
                let g = tiny();
                let r = Router::new(&g, 0, 0.);
                let a = Stop {
                    node: 0,
                    mode: 0,
                    meeting: 0,
                };
                let b = Stop { meeting: 1, ..a };
                assert!(
                    r.plan(&Itinerary {
                        routes: vec![vec![a, b].into(), vec![b, a].into()]
                    })
                    .is_none()
                );
            }
            #[test]
            fn finishing_search_preserves_legality_and_improves_objective() {
                let p = generator::generate(2, None).unwrap().problem;
                let mut kinds = vec![0; p.agents.len()];
                kinds[0] = 1;
                let g = Game::new(p, &kinds);
                let router = Router::new(&g, 0, 0.06);
                let empty = Itinerary::empty(g.agents.len());
                let best = router.polish(&empty, Instant::now() + Duration::from_millis(30));
                let e = router.evaluate(&best, None);
                assert_eq!(e.violation, 0);
                assert!(e.value > router.evaluate(&empty, None).value);
                let plan = router.plan(&best).unwrap();
                let mut actual = g.clone();
                actual.play_day(plan).unwrap();
                assert_eq!(actual.score.udon, e.udon);
            }
            #[test]
            fn all_32_spots_and_eight_patrols_respect_stock_limits() {
                let p = Problem {
                    starts_at: 0,
                    day_seconds: vec![60],
                    day_steps: vec![256],
                    map: Map {
                        width: 8,
                        height: 8,
                        cells: vec![vec![0; 8]; 8],
                    },
                    spots: (0..32)
                        .map(|i| Spot {
                            brand: 100 + i * 7,
                            pos: 8 + i,
                            stocks: (i % 8 + 1) as u32,
                        })
                        .collect(),
                    agents: (0..8).collect(),
                    fuel_limits: 1000,
                    players: 1,
                    busy_threshold: 1,
                    jammed_threshold: 3,
                };
                p.validate().unwrap();
                let mut game = Game::new(p, &[0; 8]);
                let router = Router::new(&game, 0, 0.);
                let route: Vec<_> = (0..32).map(|i| Stop::visit(i, 0)).collect();
                let solution = Itinerary {
                    routes: vec![Route::from(route); 8],
                };
                let e = router.evaluate(&solution, None);
                assert_eq!(e.daily, 32);
                assert_eq!(e.udon, 144);
                assert_eq!(e.masks, [u32::MAX; 8]);
                let plan = router.plan(&solution).unwrap();
                game.play_day(plan).unwrap();
                assert_eq!(game.score.daily_brands, 32);
                assert_eq!(game.score.udon, 144);
            }
            #[cfg(target_arch = "x86_64")]
            #[test]
            fn popcnt_evaluation_matches_portable_evaluation() {
                #[target_feature(enable = "popcnt")]
                unsafe fn accelerated(router: &Router, solution: &Itinerary) -> Evaluation {
                    router.evaluate(solution, None)
                }
                if !std::is_x86_feature_detected!("popcnt") {
                    return;
                }
                let p = generator::generate(2, None).unwrap().problem;
                let mut kinds = vec![0; p.agents.len()];
                kinds[0] = 1;
                let game = Game::new(p, &kinds);
                let router = Router::new(&game, 0, 0.06);
                let mut rng = Random::new(173);
                let mut solution = Itinerary::empty(game.agents.len());
                for _ in 0..10000 {
                    router.mutate(&mut solution, &mut rng, None);
                    let portable = router.evaluate(&solution, None);
                    let fast = unsafe { accelerated(&router, &solution) };
                    assert_eq!(portable.value, fast.value);
                    assert_eq!(portable.violation, fast.violation);
                    assert_eq!(portable.masks, fast.masks);
                    assert_eq!(portable.fuel, fast.fuel);
                }
            }
            #[cfg(target_arch = "x86_64")]
            #[test]
            #[ignore = "manual throughput measurement"]
            fn measure_portable_and_popcnt_search() {
                if !std::is_x86_feature_detected!("popcnt") {
                    return;
                }
                let p = generator::generate(2, None).unwrap().problem;
                let mut kinds = vec![0; p.agents.len()];
                kinds[0] = 1;
                let game = Game::new(p, &kinds);
                let mut router = Router::new(&game, 0, 0.06);
                router.perturb_visits(&mut Random::new(99), 0.25);
                for _ in 0..3 {
                    eprintln!("portable");
                    let result = router.improve_inner(
                        None,
                        Instant::now() + Duration::from_millis(300),
                        &mut Random::new(17),
                    );
                    eprintln!(
                        "route_stops={:?}",
                        result
                            .routes
                            .iter()
                            .map(|route| route.len())
                            .collect::<Vec<_>>()
                    );
                    eprintln!("popcnt");
                    let result = unsafe {
                        router.improve_popcnt(
                            None,
                            Instant::now() + Duration::from_millis(300),
                            &mut Random::new(17),
                        )
                    };
                    eprintln!(
                        "route_stops={:?}",
                        result
                            .routes
                            .iter()
                            .map(|route| route.len())
                            .collect::<Vec<_>>()
                    );
                }
            }
            #[test]
            fn randomized_routes_match_independent_simulation() {
                let mut rng = Random::new(8127);
                let mut checked = 0;
                for seed in [0, 2, 7, 21, 38, 51] {
                    let p = generator::generate(seed, None).unwrap().problem;
                    let mut kinds = vec![0; p.agents.len()];
                    kinds[0] = 1;
                    let mut g = Game::new(p, &kinds);
                    for _ in 0..2 {
                        let mut r = Router::new(&g, 0, 0.06);
                        let targets: Vec<_> = (0..g.agents.len())
                            .map(|a| Some(g.problem.spots[a % g.problem.spots.len()].pos))
                            .collect();
                        r.guide_next_day(&targets, 0.08);
                        r.perturb_visits(&mut rng, 0.25);
                        let mut cur = Itinerary::empty(g.agents.len());
                        for i in 0..20000 {
                            let mut candidate = cur.clone();
                            r.mutate(&mut candidate, &mut rng, Some(&r.evaluate(&cur, None)));
                            let e = r.evaluate(&candidate, None);
                            if e.violation != 0 {
                                continue;
                            }
                            if i % 31 == 0 {
                                let plan = r.plan(&candidate).unwrap();
                                let mut compacted = candidate.clone();
                                r.compact(&mut compacted);
                                assert_eq!(plan, r.plan(&compacted).unwrap());
                                let mut actual = g.clone();
                                actual.play_day(plan).unwrap();
                                assert_eq!(actual.score.udon - g.score.udon, e.udon);
                                assert_eq!(
                                    actual.score.daily_brands - g.score.daily_brands,
                                    e.daily as usize
                                );
                                for (a, car) in actual.agents.iter().enumerate() {
                                    assert_eq!(car.pos, r.net.positions[e.ends[a] as usize]);
                                    if car.kind == 0 {
                                        assert!(car.fuel as i32 >= e.fuel[a]);
                                    }
                                }
                                checked += 1;
                            }
                            if e.value >= r.evaluate(&cur, None).value || rng.below(10) == 0 {
                                cur = candidate;
                            }
                        }
                        let plan = r.plan(&cur).unwrap();
                        g.play_day(plan).unwrap();
                    }
                }
                assert!(checked > 100);
            }
        }
    }

    // 元ファイル: solver/strategy.rs
    pub mod strategy {
        use super::random::Random;
        use super::routing::{Itinerary, Router};
        use std::time::{Duration, Instant};
        use tester::engine::Game;
        use tester::model::{DayReplay, Event, Plan, Problem};

        #[derive(Clone)]
        pub struct SavedDay {
            pub route: Itinerary,
            pub positions: Vec<usize>,
            pub plan: Plan,
        }
        #[derive(Clone, Default)]
        pub struct Memory {
            pub days: Vec<SavedDay>,
            pub known: u32,
        }
        impl Memory {
            pub fn observe(&mut self, p: &Problem, replay: &DayReplay) {
                self.known |= collected(p, replay);
                if !self.days.is_empty() {
                    self.days.remove(0);
                }
            }
        }
        fn collected(p: &Problem, replay: &DayReplay) -> u32 {
            let mut ids = Vec::new();
            for spot in &p.spots {
                if !ids.contains(&spot.brand) {
                    ids.push(spot.brand);
                }
            }
            let mut bits = 0;
            for frame in &replay.frames {
                for event in &frame.events {
                    if let Event::Collect { brand, .. } = event {
                        bits |= 1 << ids.iter().position(|b| b == brand).unwrap();
                    }
                }
            }
            bits
        }
        fn transplant(router: &Router, saved: &SavedDay) -> Itinerary {
            let mut route = saved.route.clone();
            for r in &mut route.routes {
                for stop in r {
                    let old = saved.positions[stop.node as usize];
                    stop.node = router
                        .net
                        .positions
                        .iter()
                        .position(|&p| p == old)
                        .unwrap_or_else(|| {
                            router
                                .game
                                .problem
                                .spots
                                .iter()
                                .enumerate()
                                .min_by_key(|(_, s)| {
                                    let w = router.game.problem.map.width;
                                    (s.pos % w).abs_diff(old % w) + (s.pos / w).abs_diff(old / w)
                                })
                                .unwrap()
                                .0
                        }) as u8;
                }
            }
            route
        }
        #[derive(Clone)]
        struct Trajectory {
            days: Vec<SavedDay>,
            value: f64,
        }
        fn value(g: &Game, fuel_weight: f64) -> f64 {
            let mut v = g.score.unique_brands as f64 * 10_000_000.
                + g.score.daily_brands as f64 * 10_000.
                + g.score.udon as f64;
            if g.day < g.problem.day_steps.len() {
                let need = g.problem.day_steps[g.day];
                for a in &g.agents {
                    if a.kind == 0 {
                        let capacity = if g.agents.iter().any(|car| car.kind == 1) {
                            need
                        } else {
                            need * 2
                        };
                        v += fuel_weight * a.fuel.min(capacity) as f64;
                    }
                }
            }
            v
        }
        struct SearchOptions {
            horizon: usize,
            duration: Duration,
            deadline: Instant,
            fuel_weight: f64,
            guidance: f64,
            perturbation: f64,
            replay_prefix: usize,
        }
        fn rollout(
            game: &Game,
            known: u32,
            warm: &[SavedDay],
            options: SearchOptions,
            rng: &mut Random,
        ) -> Option<Trajectory> {
            let SearchOptions {
                horizon,
                duration,
                deadline,
                fuel_weight: weight,
                guidance,
                perturbation,
                replay_prefix,
            } = options;
            let mut g = game.clone();
            let mut known = known;
            let mut days = Vec::new();
            for day in 0..horizon {
                if Instant::now() + Duration::from_millis(2) >= deadline {
                    return None;
                }
                if day < replay_prefix {
                    let saved = warm.get(day)?;
                    let replay = g.play_day(saved.plan.clone()).ok()?;
                    known |= collected(&g.problem, &replay);
                    days.push(saved.clone());
                    continue;
                }
                let mut router = Router::new(&g, known, weight);
                router.perturb_visits(rng, perturbation);
                if guidance > 0.
                    && let Some(next) = warm.get(day + 1)
                {
                    let targets: Vec<_> = next
                        .route
                        .routes
                        .iter()
                        .map(|route| route.first().map(|s| next.positions[s.node as usize]))
                        .collect();
                    router.guide_next_day(&targets, guidance);
                }
                let initial = warm.get(day).map(|s| transplant(&router, s)).or_else(|| {
                    days.last().map(|previous| {
                        let mut reversed = transplant(&router, previous);
                        for route in &mut reversed.routes {
                            route.reverse();
                        }
                        reversed
                    })
                });
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining < Duration::from_millis(1) {
                    return None;
                }
                let requested = if day == 0 {
                    duration
                } else {
                    duration.mul_f64(0.6)
                };
                let allotted = requested.min(remaining.div_f64((horizon - day) as f64));
                let route = router.quick(initial.as_ref(), allotted, rng);
                let plan = router.plan(&route)?;
                let saved = SavedDay {
                    route,
                    positions: router.net.positions.clone(),
                    plan: plan.clone(),
                };
                let replay = match g.play_day(plan) {
                    Ok(replay) => replay,
                    Err(error) => {
                        eprintln!("candidate invalid: {}", error.message);
                        return None;
                    }
                };
                known |= collected(&g.problem, &replay);
                days.push(saved);
            }
            Some(Trajectory {
                days,
                value: value(&g, 0.06),
            })
        }
        fn preserved(game: &Game, warm: &[SavedDay], horizon: usize) -> Option<Trajectory> {
            if warm.len() < horizon {
                return None;
            }
            let mut g = game.clone();
            for saved in warm.iter().take(horizon) {
                g.play_day(saved.plan.clone()).ok()?;
            }
            Some(Trajectory {
                days: warm[..horizon].to_vec(),
                value: value(&g, 0.06),
            })
        }

        fn swap_roles(days: &[SavedDay], a: usize, b: usize) -> Vec<SavedDay> {
            let mut mapped = days.to_vec();
            for day in &mut mapped {
                day.route.routes.swap(a, b);
                day.plan.swap(a, b);
            }
            mapped
        }
        fn retain_candidates(candidates: &mut Vec<(Vec<u8>, Trajectory)>) {
            candidates.sort_by(|a, b| b.1.value.total_cmp(&a.1.value));
            let mut kept = Vec::new();
            for count in [0, 1, 2] {
                if let Some(i) = candidates
                    .iter()
                    .position(|(k, _)| k.iter().filter(|&&v| v == 1).count() == count)
                {
                    kept.push(candidates.remove(i));
                }
            }
            kept.extend(candidates.drain(..).take(5 - kept.len()));
            kept.sort_by(|a, b| b.1.value.total_cmp(&a.1.value));
            *candidates = kept;
        }
        pub fn select(
            problem: &Problem,
            deadline: Instant,
            rng: &mut Random,
            memory: &mut Memory,
        ) -> Vec<u8> {
            let n = problem.agents.len();
            let days = problem.day_steps.len();
            let mut kinds = vec![vec![0; n]];
            if n > 1 {
                for a in 0..n {
                    let mut k = vec![0; n];
                    k[a] = 1;
                    kinds.push(k);
                }
            }
            if n >= 4 {
                for a in 0..n {
                    for b in a + 1..n {
                        let mut k = vec![0; n];
                        k[a] = 1;
                        k[b] = 1;
                        kinds.push(k);
                    }
                }
            }
            let budget = deadline
                .saturating_duration_since(Instant::now())
                .as_secs_f64();
            let initial_time =
                Duration::from_secs_f64((budget * 0.23 / (kinds.len() * days) as f64).max(0.001));
            let mut candidates: Vec<(Vec<u8>, Trajectory)> = Vec::new();
            for k in kinds {
                if let Some(t) = rollout(
                    &Game::new(problem.clone(), &k),
                    0,
                    &[],
                    SearchOptions {
                        horizon: days,
                        duration: initial_time,
                        deadline,
                        fuel_weight: 0.06,
                        guidance: 0.,
                        perturbation: 0.,
                        replay_prefix: 0,
                    },
                    rng,
                ) {
                    eprintln!("selection kinds={k:?} value={}", t.value);
                    candidates.push((k, t));
                } else {
                    break;
                }
            }
            if candidates.is_empty() {
                return vec![0; n];
            }
            retain_candidates(&mut candidates);
            let mut attempt = 0;
            while Instant::now() + Duration::from_millis(10) < deadline {
                let index = if attempt % 3 == 0 {
                    0
                } else {
                    (attempt / 3 + attempt % 3) % candidates.len()
                };
                let weight = [0.025, 0.06, 0.12, 0.0, 0.2][attempt % 5];
                let duration = initial_time.mul_f64(2. + (attempt / 5) as f64 * 0.5);
                let (mut k, t) = candidates[index].clone();
                let mut warm = t.days;
                if attempt % 3 == 2 {
                    let patrols: Vec<_> = (0..n).filter(|&a| k[a] == 0).collect();
                    let supplies: Vec<_> = (0..n).filter(|&a| k[a] == 1).collect();
                    if !patrols.is_empty() && !supplies.is_empty() {
                        let a = patrols[rng.below(patrols.len())];
                        let b = supplies[rng.below(supplies.len())];
                        k.swap(a, b);
                        warm = swap_roles(&warm, a, b);
                    }
                }
                if let Some(candidate) = rollout(
                    &Game::new(problem.clone(), &k),
                    0,
                    &warm,
                    SearchOptions {
                        horizon: days,
                        duration,
                        deadline,
                        fuel_weight: weight,
                        guidance: [0.04, 0., 0.08, 0.12, 0.][attempt % 5],
                        perturbation: [0.12, 0., 0.25, 0.5, 0.][attempt % 5],
                        replay_prefix: 0,
                    },
                    rng,
                ) {
                    if let Some(existing) = candidates.iter_mut().find(|(kinds, _)| kinds == &k) {
                        if candidate.value > existing.1.value {
                            existing.1 = candidate;
                        }
                    } else {
                        candidates.push((k, candidate));
                    }
                    retain_candidates(&mut candidates);
                } else {
                    break;
                }
                attempt += 1;
            }
            let (k, best) = candidates.remove(0);
            eprintln!("selected kinds={k:?} forecast={}", best.value);
            memory.days = best.days;
            memory.known = 0;
            k
        }
        pub fn plan(game: &Game, deadline: Instant, rng: &mut Random, memory: &mut Memory) -> Plan {
            let horizon = 3.min(game.problem.day_steps.len() - game.day);
            let budget = deadline
                .saturating_duration_since(Instant::now())
                .as_secs_f64();
            let mut best = preserved(game, &memory.days, horizon);
            if best.is_none() && !memory.days.is_empty() {
                best = rollout(
                    game,
                    memory.known,
                    &memory.days,
                    SearchOptions {
                        horizon,
                        duration: Duration::from_secs_f64(
                            (budget / (horizon as f64 * 12.)).max(0.001),
                        ),
                        deadline,
                        fuel_weight: 0.06,
                        guidance: 0.,
                        perturbation: 0.,
                        replay_prefix: memory.days.len().min(horizon),
                    },
                    rng,
                );
            }
            let mut attempts = 0;
            while Instant::now() + Duration::from_millis(5) < deadline {
                let weight = [0.06, 0.02, 0.12, 0.0, 0.2][attempts % 5];
                let duration =
                    Duration::from_secs_f64((budget / (horizon as f64 * 12.)).max(0.001));
                let warm = best
                    .as_ref()
                    .map_or(memory.days.as_slice(), |t| t.days.as_slice());
                let candidate = rollout(
                    game,
                    memory.known,
                    warm,
                    SearchOptions {
                        horizon,
                        duration,
                        deadline,
                        fuel_weight: weight,
                        guidance: [0., 0.04, 0.08, 0.12, 0.][attempts % 5],
                        perturbation: [0., 0.12, 0.25, 0.5, 0.][attempts % 5],
                        replay_prefix: 0,
                    },
                    rng,
                );
                let Some(candidate) = candidate else {
                    break;
                };
                if best.as_ref().is_none_or(|b| candidate.value > b.value) {
                    best = Some(candidate);
                }
                attempts += 1;
            }
            if let Some(best) = best {
                eprintln!(
                    "chosen day={} attempts={} forecast={}",
                    game.day, attempts, best.value
                );
                let plan = best.days[0].plan.clone();
                memory.days = best.days;
                plan
            } else {
                eprintln!("no complete candidate; using legal waiting plan");
                memory.days.clear();
                game.waiting_plan()
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use tester::generator;
            #[test]
            fn cached_two_days_are_replayed_before_extending_the_horizon() {
                let p = generator::generate(0, None).unwrap().problem;
                let mut kinds = vec![0; p.agents.len()];
                kinds[0] = 1;
                let game = Game::new(p, &kinds);
                let mut rng = Random::new(112);
                let first = rollout(
                    &game,
                    0,
                    &[],
                    SearchOptions {
                        horizon: 2,
                        duration: Duration::from_millis(10),
                        deadline: Instant::now() + Duration::from_secs(2),
                        fuel_weight: 0.06,
                        guidance: 0.,
                        perturbation: 0.,
                        replay_prefix: 0,
                    },
                    &mut rng,
                )
                .unwrap();
                let extended = rollout(
                    &game,
                    0,
                    &first.days,
                    SearchOptions {
                        horizon: 3,
                        duration: Duration::from_millis(10),
                        deadline: Instant::now() + Duration::from_secs(2),
                        fuel_weight: 0.06,
                        guidance: 0.,
                        perturbation: 0.,
                        replay_prefix: 2,
                    },
                    &mut rng,
                )
                .unwrap();
                assert_eq!(extended.days.len(), 3);
                for day in 0..2 {
                    assert_eq!(first.days[day].plan, extended.days[day].plan);
                }
                let mut replay = game.clone();
                for saved in &extended.days {
                    replay.play_day(saved.plan.clone()).unwrap();
                }
                assert_eq!(extended.value, value(&replay, 0.06));
            }
            #[test]
            fn role_permutation_preserves_a_plan_when_agent_states_are_permuted() {
                let p = generator::generate(2, None).unwrap().problem;
                let mut kinds = vec![0; p.agents.len()];
                kinds[0] = 1;
                let g = Game::new(p, &kinds);
                let router = Router::new(&g, 0, 0.06);
                let route = router.quick(None, Duration::from_millis(30), &mut Random::new(6));
                let plan = router.plan(&route).unwrap();
                let saved = SavedDay {
                    route,
                    plan: plan.clone(),
                    positions: router.net.positions.clone(),
                };
                let mut original = g.clone();
                original.play_day(plan).unwrap();
                let mut permuted = g.clone();
                permuted.agents.swap(0, 1);
                permuted.problem.agents.swap(0, 1);
                let mapped = swap_roles(&[saved], 0, 1);
                let router = Router::new(&permuted, 0, 0.06);
                let plan = router.plan(&transplant(&router, &mapped[0])).unwrap();
                permuted.play_day(plan).unwrap();
                original.agents.swap(0, 1);
                assert_eq!(original.agents, permuted.agents);
                assert_eq!(original.score, permuted.score);
                assert_eq!(original.traffic(), permuted.traffic());
            }
        }
    }

    #[cfg(test)]
    // 元ファイル: solver/tests.rs
    mod tests {
        use super::random::Random;
        #[test]
        fn random_seed_is_repeatable_and_bounded() {
            let mut a = Random::new(0);
            let mut b = Random::new(0);
            for _ in 0..1000 {
                assert_eq!(a.next(), b.next());
            }
            for n in 1..100 {
                for _ in 0..100 {
                    assert!(a.below(n) < n);
                }
            }
        }
    }
}

use solver::random::Random;
use solver::strategy::{self, Memory};
use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};
use tester::engine::Game;
use tester::model::{Plan, Request, Response};

fn main() {
    let seed = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut rng = Random::new(seed);
    let mut memory = Memory::default();
    let mut game: Option<Game> = None;
    let mut pending: Option<Plan> = None;
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let started = Instant::now();
        let request: Request = serde_json::from_str(&line).expect("JSON Lines request");
        let reserve = (request.time_limit_ms / 100).clamp(15, 700);
        let deadline =
            started + Duration::from_millis(request.time_limit_ms.saturating_sub(reserve));
        let answer = if let Some(problem) = request.problem {
            let kinds = strategy::select(&problem, deadline, &mut rng, &mut memory);
            game = Some(Game::new(problem, &kinds));
            serde_json::to_value(kinds).unwrap()
        } else {
            let g = game.as_mut().expect("selection first");
            let state = request.state.expect("state");
            if let Some(previous) = pending.take() {
                let accepted = request.previous_accepted != Some(false);
                let replay = g
                    .play_day(if accepted { previous } else { g.waiting_plan() })
                    .expect("previous accepted plan");
                memory.observe(&g.problem, &replay);
                if !accepted {
                    memory.days.clear();
                }
            } else if g.day == 0 && request.previous_accepted == Some(false) {
                *g = Game::new(
                    g.problem.clone(),
                    &state.agents.iter().map(|a| a.kind).collect::<Vec<_>>(),
                );
                memory = Memory::default();
            }
            assert_eq!(g.day, state.day);
            assert_eq!(g.agents, state.agents);
            assert_eq!(g.traffic(), state.traffics);
            let plan = strategy::plan(g, deadline, &mut rng, &mut memory);
            g.clone()
                .play_day(plan.clone())
                .expect("independent final validation");
            pending = Some(plan.clone());
            serde_json::to_value(plan).unwrap()
        };
        serde_json::to_writer(
            &mut output,
            &Response {
                request_id: request.request_id,
                answer,
            },
        )
        .unwrap();
        writeln!(output).unwrap();
        output.flush().unwrap();
        eprintln!(
            "request={} elapsed_ms={}",
            request.request_id,
            started.elapsed().as_millis()
        );
    }
}
