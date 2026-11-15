// solver01: 基準実装。出典コミット: cc3277fa2d95ea7494a4d552429d2c862a9787a7
// アルゴリズム・設定・対応表: solver/docs/rebuild-experiment-imak/README.md
// 詳細: solver/docs/rebuild-experiment-imak/solver01/algorithm.md
// 記録用の単一ファイル。開発用の main.rs・solver/*.rs は別途維持する。
// 通信・探索・テストを収録する。tester と serde_json は Cargo 依存。
// 元の外部モジュールをインライン化し、探索処理・定数・テストを保持している。

// 元ファイル: solver/mod.rs
mod solver {
    // 元ファイル: solver/alns.rs
    mod alns {
        use super::*;
        use std::collections::HashSet;

        type ForecastKey = (u32, u32, u32, u32, (usize, u32, u64), i32);

        #[derive(Clone)]
        pub(super) struct PoolEntry {
            sig: u64,
            sim: Sim,
            tours: Tours,
        }
        pub(super) struct Pool {
            pub active: bool,
            pub entries: Vec<PoolEntry>,
            reference: Option<(u32, u32)>,
            cap: usize,
            diverse: bool,
            daily_slack: u32,
        }
        impl Pool {
            pub(super) fn add(&mut self, planner: &Planner, sim: &Sim, tours: &Tours) {
                if !self.active {
                    return;
                }
                let key = (sim.newkinds, sim.claimed.count_ones());
                if self.reference.is_none_or(|r| key > r) {
                    self.reference = Some(key);
                    self.entries.retain(|e| {
                        e.sim.newkinds == key.0
                            && e.sim.claimed.count_ones() + self.daily_slack >= key.1
                    });
                }
                let reference = self.reference.unwrap();
                if key.0 != reference.0 || key.1 + self.daily_slack < reference.1 {
                    return;
                }
                let sig = planner.end_signature(sim)
                    ^ if self.diverse {
                        planner.tours_hash(tours).rotate_left(23)
                    } else {
                        0
                    };
                if let Some(entry) = self.entries.iter_mut().find(|e| e.sig == sig) {
                    if sim.balls > entry.sim.balls {
                        entry.sim = sim.clone();
                        entry.tours = tours.clone();
                    }
                    return;
                }
                let entry = PoolEntry {
                    sig,
                    sim: sim.clone(),
                    tours: tours.clone(),
                };
                if self.entries.len() < self.cap {
                    self.entries.push(entry);
                    return;
                }
                let quality = |s: &Sim| {
                    (
                        s.future
                            + if self.daily_slack > 0 {
                                s.claimed.count_ones()
                            } else {
                                0
                            },
                        s.balls,
                    )
                };
                if self.diverse {
                    let elite = (self.cap / 2).max(1);
                    self.entries
                        .sort_unstable_by_key(|e| std::cmp::Reverse(quality(&e.sim)));
                    let priority = |sig: u64| {
                        let x = (sig ^ (sig >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                        let x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
                        x ^ (x >> 31)
                    };
                    let victim = (elite..self.cap)
                        .min_by_key(|&i| priority(self.entries[i].sig))
                        .unwrap_or(self.cap - 1);
                    if quality(sim) > quality(&self.entries[elite - 1].sim)
                        || priority(sig) > priority(self.entries[victim].sig)
                    {
                        self.entries[victim] = entry;
                    }
                    return;
                }
                let worst = self
                    .entries
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, e)| quality(&e.sim))
                    .unwrap()
                    .0;
                if quality(&self.entries[worst].sim) < quality(sim) {
                    self.entries[worst] = entry;
                }
            }
        }

        #[derive(Clone)]
        pub(super) struct Weights {
            pub dw: [f64; 5],
            pub rw: [f64; 3],
            ds: [f64; 5],
            rs: [f64; 3],
            dt: [u64; 5],
            rt: [u64; 3],
        }
        impl Default for Weights {
            fn default() -> Self {
                Self {
                    dw: [1.0; 5],
                    rw: [1.0; 3],
                    ds: [0.0; 5],
                    rs: [0.0; 3],
                    dt: [0; 5],
                    rt: [0; 3],
                }
            }
        }
        impl Weights {
            pub(super) fn record(&mut self, operators: Option<(usize, usize)>, reward: f64) {
                if let Some((d, r)) = operators {
                    self.dt[d] += 1;
                    self.rt[r] += 1;
                    self.ds[d] += reward;
                    self.rs[r] += reward;
                }
            }
            pub(super) fn update(&mut self) {
                for i in 0..5 {
                    if self.dt[i] > 0 {
                        self.dw[i] = 0.7 * self.dw[i] + 0.3 * self.ds[i] / self.dt[i] as f64;
                    }
                    self.dw[i] = self.dw[i].max(0.1);
                    self.dt[i] = 0;
                    self.ds[i] = 0.0;
                }
                for i in 0..3 {
                    if self.rt[i] > 0 {
                        self.rw[i] = 0.7 * self.rw[i] + 0.3 * self.rs[i] / self.rt[i] as f64;
                    }
                    self.rw[i] = self.rw[i].max(0.1);
                    self.rt[i] = 0;
                    self.rs[i] = 0.0;
                }
            }
        }

        #[derive(Clone, Default)]
        pub(super) struct Stats {
            pub iterations: u64,
            pub evaluations: u64,
            pub memo_hits: u64,
            pub lazy_skips: u64,
            pub sessions: u64,
            pub culls: u64,
        }

        #[derive(Clone, Copy)]
        struct Verdict {
            nk: i32,
            today: i32,
            nk2: i32,
            today2: i32,
            balls: i32,
        }
        impl Default for Verdict {
            fn default() -> Self {
                Self {
                    nk: -1,
                    today: -1,
                    nk2: -1,
                    today2: -1,
                    balls: -1,
                }
            }
        }
        impl Verdict {
            fn daily_key(&self, aggregate: usize) -> (i32, i32, i32, i32) {
                if aggregate == 1 {
                    (
                        self.nk + self.nk2.max(0),
                        self.today + self.today2.max(0),
                        0,
                        0,
                    )
                } else {
                    (self.nk, self.today, self.nk2, self.today2)
                }
            }
            fn key(&self, aggregate: usize) -> ((i32, i32, i32, i32), i32) {
                (self.daily_key(aggregate), self.balls)
            }
        }

        impl Planner {
            fn end_signature(&self, sim: &Sim) -> u64 {
                let mut h = 1469598103934665603_u64;
                let mut mix = |v: u64| {
                    h ^= v.wrapping_add(0x9e3779b97f4a7c15);
                    h = h.wrapping_mul(1099511628211);
                };
                for &p in &self.patrols {
                    mix(sim.endpos[p] as u64);
                    mix(sim.endfuel[p] as u64);
                }
                for b in 0..32 {
                    if sim.claimed & (1 << b) != 0 {
                        mix(b);
                    }
                }
                h
            }
            pub(super) fn scalar(s: &Sim) -> f64 {
                s.newkinds as f64 * 500.0
                    + s.future_newkinds as f64 * 100.0
                    + s.claimed.count_ones() as f64 * 10.0
                    + s.future as f64
                    + s.balls as f64 * 0.02
            }
            fn better_session(&self, a: &Sim, b: &Sim) -> bool {
                let akey = (a.newkinds, a.claimed.count_ones());
                let bkey = (b.newkinds, b.claimed.count_ones());
                akey > bkey
                    || (akey == bkey
                        && self.config.final_continue
                        && self.day_index + 1 >= self.map.problem.day_steps.len()
                        && a.balls > b.balls)
            }
            pub(super) fn reward(&self, s: &Sim, best: &Sim) -> f64 {
                if self.better(s, best) {
                    10.0
                } else if s.newkinds == best.newkinds
                    && s.claimed.count_ones() == best.claimed.count_ones()
                    && s.future == best.future
                    && s.balls >= best.balls
                {
                    3.0
                } else {
                    0.0
                }
            }
            pub(super) fn propose(
                &mut self,
                base: &Tours,
                restart_base: &Tours,
                weights: &Weights,
            ) -> (Tours, Option<(usize, usize)>) {
                let roll = self.rng.next();
                if self.config.neighbor_mode == 1 {
                    let d = self.rng.weighted(&weights.dw);
                    let r = self.rng.weighted(&weights.rw);
                    let q = self.unified_degree();
                    let mut t = base.clone();
                    self.destroy_repair(&mut t, d, r, q, true);
                    (t, Some((d, r)))
                } else if roll < self.config.restart_prob {
                    (
                        if self.config.restart_mode != 0 {
                            self.shake(restart_base)
                        } else {
                            self.greedy_init(true)
                        },
                        None,
                    )
                } else if roll < self.config.restart_prob + self.config.mutate_prob {
                    let mut t = base.clone();
                    self.mutate(&mut t, false);
                    (t, None)
                } else {
                    let d = self.rng.weighted(&weights.dw);
                    let r = self.rng.weighted(&weights.rw);
                    let q = self.destroy_degree();
                    let mut t = base.clone();
                    self.destroy_repair(&mut t, d, r, q, false);
                    (t, Some((d, r)))
                }
            }
            pub(super) fn retry_tours(&mut self, tours: &mut Tours) {
                if self.config.retry_mode == 2 {
                    let q = self.destroy_degree();
                    let d = self.rng.index(5);
                    let r = self.rng.index(3);
                    self.destroy_repair(tours, d, r, q, false);
                } else {
                    self.mutate(tours, self.config.retry_mode == 1);
                }
            }
            pub(super) fn accept_sa(
                &mut self,
                s: &Sim,
                current: &Sim,
                fraction: f64,
                scale: f64,
            ) -> bool {
                if self.better(s, current) {
                    return true;
                }
                let delta = Self::scalar(current) - Self::scalar(s);
                let temp = self.config.sa_t0
                    * (self.config.sa_t1 / self.config.sa_t0).powf(fraction.min(1.0))
                    * scale;
                delta <= 0.0 || self.rng.next() < (-delta / temp.max(1e-9)).exp()
            }

            pub(super) fn run_alns(&mut self, init: Tours, start: Instant) -> Sim {
                let mut best = self.simulate(&init);
                self.rollout(&mut best);
                if self.patrols.is_empty() || self.map.problem.spots.is_empty() {
                    return best;
                }
                let mut best_tours = init.clone();
                let mut stats = Stats::default();
                let mut pool = Pool {
                    active: self.config.pool
                        && self.config.budget_ms >= 20_000
                        && self.config.iterations == 0
                        && self.day_index + 1 < self.map.problem.day_steps.len(),
                    entries: vec![],
                    reference: None,
                    cap: self.config.pool_cap,
                    diverse: self.config.pool_diverse,
                    daily_slack: if self.config.exact_pool
                        && config::setting("EXACT_POOL_TOTAL", 0) != 0
                    {
                        config::setting("POOL_DAILY_SLACK", 1_u32).min(self.map.brands as u32)
                    } else {
                        0
                    },
                };
                pool.add(self, &best, &init);
                let mut deadline = self.config.budget_ms;
                if pool.active {
                    let reserved = if self.config.exact_pool {
                        config::setting("EXACT_POOL_MS", 15_000.min(deadline / 3))
                    } else {
                        15_000.min(deadline / 3)
                    };
                    deadline -= reserved.min(deadline.saturating_sub(100));
                }
                if self.config.population {
                    self.run_population(
                        &init,
                        start,
                        deadline,
                        &mut best,
                        &mut best_tours,
                        &mut pool,
                        &mut stats,
                    );
                } else {
                    let num_sessions = if (self.config.budget_ms >= 20_000
                        || self.config.force_session)
                        && self.config.iterations == 0
                    {
                        (self.config.budget_ms / self.config.session_ms).max(1)
                    } else {
                        1
                    };
                    let memo_on = self.config.memo && !self.config.sa;
                    let lazy_on = self.config.lazy && !self.config.sa;
                    let mut extended = false;
                    let mut memo = HashSet::new();
                    for session in 0.. {
                        if self.expired() || (num_sessions == 1 && session >= 1) {
                            break;
                        }
                        let mut now = Self::elapsed(start);
                        if now >= deadline {
                            if pool.active
                                && pool.entries.len() < 2
                                && !extended
                                && self.config.iterations == 0
                            {
                                extended = true;
                                deadline = self.config.budget_ms;
                                now = Self::elapsed(start);
                                if now >= deadline {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        let remaining = deadline - now;
                        let budget = if num_sessions == 1 {
                            remaining
                        } else {
                            self.config.session_ms.min(remaining)
                        };
                        if budget < 100 {
                            break;
                        }
                        let mut local_tours = if session == 0 {
                            init.clone()
                        } else {
                            for _ in 0..37 {
                                self.rng.next();
                            }
                            if self.config.final_continue
                                && self.day_index + 1 >= self.map.problem.day_steps.len()
                            {
                                best_tours.clone()
                            } else if self.config.session_mode == 2 {
                                init.clone()
                            } else if self.config.session_mode != 0 {
                                self.shake(&init)
                            } else {
                                self.greedy_init(true)
                            }
                        };
                        let mut local_best = self.simulate(&local_tours);
                        self.rollout(&mut local_best);
                        pool.add(self, &local_best, &local_tours);
                        if memo_on {
                            memo.clear();
                            memo.insert(self.tours_hash(&local_tours));
                        }
                        let mut current_tours = local_tours.clone();
                        let mut current = local_best.clone();
                        let mut weights = Weights::default();
                        let session_start = Instant::now();
                        let mut session_iters = 0;
                        loop {
                            stats.iterations += 1;
                            session_iters += 1;
                            let elapsed = Self::elapsed(session_start);
                            if self.expired()
                                || (if self.config.iterations > 0 {
                                    stats.iterations >= self.config.iterations
                                } else {
                                    elapsed >= budget
                                })
                            {
                                break;
                            }
                            let base = if self.config.sa {
                                &current_tours
                            } else {
                                &local_tours
                            };
                            let (mut tours, operators) = self.propose(base, base, &weights);
                            let mut duplicate = false;
                            if memo_on {
                                duplicate = !memo.insert(self.tours_hash(&tours));
                                for _ in 0..self.config.retry {
                                    if !duplicate {
                                        break;
                                    }
                                    stats.memo_hits += 1;
                                    self.retry_tours(&mut tours);
                                    duplicate = !memo.insert(self.tours_hash(&tours));
                                }
                            }
                            if duplicate {
                                stats.memo_hits += 1;
                                weights.record(operators, 0.0);
                            } else {
                                let mut sim = self.simulate(&tours);
                                stats.evaluations += 1;
                                let lazy = lazy_on
                                    && self.config.depth > 0
                                    && sim.newkinds < local_best.newkinds;
                                if lazy {
                                    stats.lazy_skips += 1;
                                } else {
                                    self.rollout(&mut sim);
                                }
                                pool.add(self, &sim, &tours);
                                let improved = self.better(&sim, &local_best);
                                weights.record(operators, self.reward(&sim, &local_best));
                                if self.config.sa
                                    && self.accept_sa(
                                        &sim,
                                        &current,
                                        elapsed as f64 / budget.max(1) as f64,
                                        1.0,
                                    )
                                {
                                    current = sim.clone();
                                    current_tours = tours.clone();
                                }
                                if improved {
                                    local_best = sim;
                                    local_tours = tours;
                                }
                            }
                            if session_iters % 60 == 0 {
                                weights.update();
                            }
                            if stats.iterations >= 50_000_000 {
                                break;
                            }
                        }
                        let adopt = session == 0 || self.better_session(&local_best, &best);
                        if self.config.debug > 0 {
                            eprintln!(
                                "[day {} session {session}] kinds={} daily={} balls={} adopt={adopt}",
                                self.day_index,
                                local_best.newkinds,
                                local_best.claimed.count_ones(),
                                local_best.balls
                            );
                        }
                        if adopt {
                            best = local_best;
                            best_tours = local_tours;
                        }
                        stats.sessions += 1;
                    }
                }
                let (rounds, switched) =
                    self.verify_pool(&mut pool, start, &mut best, &mut best_tours);
                if self.config.debug > 0 {
                    eprintln!(
                        "[day {}] iters={} evals={} memo_hits={} lazy_skips={} sessions={} culls={} pool={} rounds={rounds} switch={switched} kinds={} daily={} balls={} future={}",
                        self.day_index,
                        stats.iterations,
                        stats.evaluations,
                        stats.memo_hits,
                        stats.lazy_skips,
                        stats.sessions,
                        stats.culls,
                        pool.entries.len(),
                        best.newkinds,
                        best.claimed.count_ones(),
                        best.balls,
                        best.future
                    );
                }
                self.last_tours = best_tours;
                best
            }

            fn verify_pool(
                &mut self,
                pool: &mut Pool,
                start: Instant,
                best: &mut Sim,
                best_tours: &mut Tours,
            ) -> (u64, bool) {
                if !pool.active || pool.entries.len() < 2 {
                    return (0, false);
                }
                if self.config.exact_pool {
                    return self.verify_exact_pool(pool, start, best, best_tours);
                }
                let sig = self.end_signature(best);
                if !pool.entries.iter().any(|e| e.sig == sig) {
                    let entry = PoolEntry {
                        sig,
                        sim: best.clone(),
                        tours: best_tours.clone(),
                    };
                    if pool.entries.len() < pool.cap {
                        pool.entries.push(entry);
                    } else {
                        *pool.entries.last_mut().unwrap() = entry;
                    }
                }
                let steps = self.map.problem.day_steps[self.day_index + 1] as i32;
                let steps2 = if self.config.verify_depth >= 2 {
                    self.map
                        .problem
                        .day_steps
                        .get(self.day_index + 2)
                        .copied()
                        .unwrap_or(0) as i32
                } else {
                    0
                };
                let mut verdicts = vec![Verdict::default(); pool.entries.len()];
                let mut rounds = 0_u64;
                while rounds < 100_000_000
                    && Self::elapsed(start)
                        < self.config.budget_ms.saturating_sub(self.config.phasec_ms)
                    && !self.expired()
                {
                    for (i, entry) in pool.entries.iter().enumerate() {
                        let seed = 0x51ed270b_u32
                            .wrapping_add((self.day_index as u32).wrapping_mul(0x9e3779b9))
                            .wrapping_add((rounds as u32).wrapping_mul(0x85ebca6b))
                            .wrapping_add(self.base_seed.wrapping_mul(0xc2b2ae35));
                        self.rng = Prng::new(if seed == 0 { 1 } else { seed });
                        let mut pos = entry.sim.endpos.clone();
                        let mut fuel = entry.sim.endfuel.clone();
                        let mut brands = self.brands_ever | entry.sim.claimed;
                        let escorts = self.config.rollout_escorts.then_some(&entry.sim);
                        let r = self.greedy_day(
                            &mut pos,
                            &mut fuel,
                            &mut brands,
                            steps,
                            rounds > 0,
                            escorts,
                        );
                        let mut cand = Verdict {
                            nk: r.newkinds as i32,
                            today: r.claimed as i32,
                            balls: r.balls,
                            ..Verdict::default()
                        };
                        if steps2 > 0 {
                            let r = self.greedy_day(
                                &mut pos,
                                &mut fuel,
                                &mut brands,
                                steps2,
                                rounds > 0,
                                escorts,
                            );
                            cand.nk2 = r.newkinds as i32;
                            cand.today2 = r.claimed as i32;
                            cand.balls += r.balls;
                        }
                        if cand.key(self.config.verify_agg)
                            > verdicts[i].key(self.config.verify_agg)
                        {
                            verdicts[i] = cand;
                        }
                    }
                    rounds += 1;
                }
                if rounds < 8 {
                    return (rounds, false);
                }
                let incumbent = pool.entries.iter().position(|e| e.sig == sig).unwrap();
                let mut bi = incumbent;
                let inc_key = verdicts[incumbent].daily_key(self.config.verify_agg);
                let inc_balls = pool.entries[incumbent].sim.balls + verdicts[incumbent].balls;
                for i in 0..pool.entries.len() {
                    if i == incumbent || verdicts[i].nk < 0 {
                        continue;
                    }
                    let key = verdicts[i].daily_key(self.config.verify_agg);
                    let balls = pool.entries[i].sim.balls + verdicts[i].balls;
                    if key <= inc_key
                        && !(self.config.atb
                            && self.map.brands < 20
                            && key == inc_key
                            && balls > inc_balls)
                    {
                        continue;
                    }
                    if bi == incumbent
                        || (key, balls)
                            > (
                                verdicts[bi].daily_key(self.config.verify_agg),
                                pool.entries[bi].sim.balls + verdicts[bi].balls,
                            )
                    {
                        bi = i;
                    }
                }
                *best = pool.entries[bi].sim.clone();
                *best_tours = pool.entries[bi].tours.clone();
                if self.config.phasec_ms > 0 {
                    let mut memo = HashSet::new();
                    memo.insert(self.tours_hash(best_tours));
                    while Self::elapsed(start) < self.config.budget_ms && !self.expired() {
                        let mut cand = best_tours.clone();
                        let d = self.rng.index(5);
                        let r = self.rng.index(3);
                        let q = self.unified_degree();
                        self.destroy_repair(&mut cand, d, r, q, true);
                        if !memo.insert(self.tours_hash(&cand)) {
                            continue;
                        }
                        let mut sim = self.simulate(&cand);
                        if sim.newkinds < best.newkinds
                            || sim.claimed.count_ones() < best.claimed.count_ones()
                        {
                            continue;
                        }
                        self.rollout(&mut sim);
                        if self.better(&sim, best) {
                            *best = sim;
                            *best_tours = cand;
                        }
                    }
                }
                (rounds, bi != incumbent)
            }
        }

        impl Planner {
            fn verify_exact_pool(
                &mut self,
                pool: &Pool,
                start: Instant,
                best: &mut Sim,
                best_tours: &mut Tours,
            ) -> (u64, bool) {
                self.verify_exact_pool_mode(
                    pool,
                    start,
                    best,
                    best_tours,
                    config::setting("EXACT_POOL_RACE", 0) != 0,
                )
            }

            fn verify_exact_pool_mode(
                &mut self,
                pool: &Pool,
                start: Instant,
                best: &mut Sim,
                best_tours: &mut Tours,
                racing: bool,
            ) -> (u64, bool) {
                let mut entries = vec![(best.clone(), best_tours.clone())];
                for e in &pool.entries {
                    if e.tours != *best_tours {
                        entries.push((e.sim.clone(), e.tours.clone()));
                    }
                }
                let mut best_key = None;
                let mut winner = 0;
                let mut evaluated = 0;
                let mut winning_warm = None;
                let mut winning_forecast = vec![];
                let mut winning_reserve = (0, 0, 0);
                let mut winning_sim = best.clone();
                let mut winning_tours = best_tours.clone();
                let depth = config::setting("EXACT_POOL_DEPTH", 2usize)
                    .max(1)
                    .min(self.map.problem.day_steps.len() - self.day_index - 1);
                let total_score = config::setting("EXACT_POOL_TOTAL", 0) != 0;
                let mut order: Vec<_> = (0..entries.len()).collect();
                let mut rankings: Vec<Option<ForecastKey>> = vec![None; entries.len()];
                for pass in 0..if racing { 2 } else { 1 } {
                    let total_remaining =
                        self.config.budget_ms.saturating_sub(Self::elapsed(start));
                    let round_budget = if racing && pass == 0 {
                        total_remaining / 3
                    } else {
                        total_remaining
                    };
                    let per_day_ms = (round_budget / (order.len().max(1) * depth.max(1)) as u64)
                        .clamp(50, config::setting("EXACT_POOL_DAY_MS", 1500_u64).max(50));
                    for &i in &order {
                        if self.expired() || Self::elapsed(start) + 100 > self.config.budget_ms {
                            break;
                        }
                        let (mut sim, tours) = entries[i].clone();
                        self.fill_supply(&mut sim);
                        let Some(actual) = self.replay(&sim.ops, &self.apos, &self.afuel) else {
                            continue;
                        };
                        let mut child = Planner::new(self.map.problem.clone(), self.base_seed);
                        child.config = self.config.clone();
                        child.config.pool = false;
                        child.config.depth = self.config.exact_pool_search_depth;
                        child.config.iterations = 0;
                        child.config.budget_ms = per_day_ms;
                        child.config.beam_ms = Some((per_day_ms / 5).min(100));
                        child.config.force_session = false;
                        child.config.debug = 0;
                        child.brands_ever = self.brands_ever | actual.claimed;
                        child.kinds = self.kinds.clone();
                        let mut history = self.road_history.clone();
                        history.push(actual.stay);
                        let mut agents: Vec<_> = (0..self.na)
                            .map(|p| tester::model::Agent {
                                kind: self.kinds[p],
                                pos: actual.pos[p],
                                fuel: actual.fuel[p] as u32,
                            })
                            .collect();
                        let mut daily = actual.claimed.count_ones();
                        let today = daily;
                        let mut balls = actual.balls;
                        let mut future_counts = vec![];
                        let mut warm = None;
                        for offset in 1..=depth {
                            let day = self.day_index + offset;
                            if day >= self.map.problem.day_steps.len() || self.expired() {
                                break;
                            }
                            let observation = self.horizon_observation(
                                day,
                                std::mem::take(&mut agents),
                                &history,
                            );
                            // Future searches share the parent's deadline, including preprocessing.
                            child.hard_deadline = Some(
                                (Instant::now() + Duration::from_millis(per_day_ms))
                                    .min(start + Duration::from_millis(self.config.budget_ms))
                                    .min(self.hard_deadline.unwrap_or(
                                        start + Duration::from_millis(self.config.budget_ms),
                                    )),
                            );
                            let ops = child.plan_day_local(
                                &observation,
                                self.map.problem.day_steps[day] as i32,
                            );
                            let Some(result) = child.replay(&ops, &child.apos, &child.afuel) else {
                                break;
                            };
                            daily += result.claimed.count_ones();
                            future_counts.push(result.claimed.count_ones());
                            balls += result.balls;
                            if offset == 1 {
                                warm = Some(WarmStart {
                                    observation,
                                    tours: child.last_tours.clone(),
                                    targets: child
                                        .supplies
                                        .iter()
                                        .map(|&p| child.supply_target[p])
                                        .collect(),
                                    order: child.order.clone(),
                                });
                            }
                            history.push(result.stay);
                            agents = (0..self.na)
                                .map(|p| tester::model::Agent {
                                    kind: self.kinds[p],
                                    pos: result.pos[p],
                                    fuel: result.fuel[p] as u32,
                                })
                                .collect();
                        }
                        // Compare equally long forecasts; a truncated candidate has no comparable score.
                        if future_counts.len() != depth {
                            continue;
                        }
                        let reserve = self.forecast_reserve(&agents, depth);
                        let key = if total_score {
                            (child.brands_ever.count_ones(), daily, 0, 0, reserve, balls)
                        } else {
                            (
                                (actual.claimed & !self.brands_ever).count_ones(),
                                today,
                                child.brands_ever.count_ones(),
                                daily,
                                reserve,
                                balls,
                            )
                        };
                        if self.config.debug > 0 {
                            eprintln!(
                                "[exact pool] day={} pass={pass} entry={i} today={today} next={future_counts:?}",
                                self.day_index
                            );
                        }
                        evaluated += 1;
                        rankings[i] = Some(rankings[i].map_or(key, |previous| previous.max(key)));
                        if best_key.is_none_or(|b| key > b) {
                            best_key = Some(key);
                            winner = i;
                            winning_warm = warm;
                            winning_forecast = future_counts;
                            winning_reserve = reserve;
                            winning_sim = sim;
                            winning_tours = tours;
                        }
                    }
                    if racing && pass == 0 {
                        order.retain(|&i| rankings[i].is_some());
                        order.sort_by_key(|&i| std::cmp::Reverse(rankings[i]));
                        order.truncate(config::setting("EXACT_POOL_FINALISTS", 4_usize).max(1));
                        // Re-evaluate the incumbent too, so refinement cannot favor only challengers.
                        if rankings[0].is_some() && !order.contains(&0) {
                            if let Some(last) = order.last_mut() {
                                *last = 0;
                            }
                        }
                        if order.is_empty() {
                            break;
                        }
                    }
                }
                if self.config.debug > 0 && best_key.is_some() {
                    eprintln!(
                        "[exact selected] day={} entry={winner} next={winning_forecast:?} search_depth={} reserve={winning_reserve:?}",
                        self.day_index, self.config.exact_pool_search_depth
                    );
                }
                *best = winning_sim;
                *best_tours = winning_tours;
                self.warm_start = winning_warm;
                (evaluated, winner != 0)
            }

            fn forecast_reserve(
                &self,
                agents: &[tester::model::Agent],
                depth: usize,
            ) -> (usize, u32, u64) {
                // Fuel has no value after the last day. Before then, avoid leaving a
                // patrol unable to start another move when forecast coverage is tied.
                if !self.config.exact_pool_reserve
                    || self.day_index + depth + 1 >= self.map.problem.day_steps.len()
                {
                    return (0, 0, 0);
                }
                let mut mobile = 0;
                let mut minimum = None;
                let mut total = 0_u64;
                for agent in agents.iter().filter(|a| a.kind == 0) {
                    mobile += usize::from(agent.fuel >= self.map.fuel_cost(agent.pos) as u32);
                    minimum = Some(minimum.map_or(agent.fuel, |old: u32| old.min(agent.fuel)));
                    total += u64::from(agent.fuel);
                }
                (mobile, minimum.unwrap_or(0), total)
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            fn check_pool_forecast(racing: bool, search_depth: usize, reserve: bool) {
                let problem = tester::generator::generate(0, None).unwrap().problem;
                let mut game = tester::engine::Game::new(problem.clone(), &[0, 1, 0, 0]);
                let mut planner = Planner::new(problem, 5);
                planner.config.budget_ms = 1800;
                planner.config.beam = false;
                planner.config.exact_pool_search_depth = search_depth;
                planner.config.exact_pool_reserve = reserve;
                planner.prepare(&game.observation(0), 32);
                let targets = planner.select_supply_targets();
                planner.configure_supply(&targets);
                let mut tours = planner.greedy_init(false);
                let mut alternative = tours.clone();
                alternative[0].reverse();
                alternative[0].push(0);
                let mut best = planner.simulate(&tours);
                let pool = Pool {
                    active: true,
                    entries: vec![PoolEntry {
                        sig: 1,
                        sim: planner.simulate(&alternative),
                        tours: alternative,
                    }],
                    reference: None,
                    cap: 12,
                    diverse: true,
                    daily_slack: 0,
                };
                planner.set_deadline(4000);
                let (evaluated, _) = planner.verify_exact_pool_mode(
                    &pool,
                    Instant::now(),
                    &mut best,
                    &mut tours,
                    racing,
                );
                assert!(evaluated >= if racing { 3 } else { 2 });
                game.play_day(
                    best.ops
                        .iter()
                        .map(|p| p.iter().copied().map(i64::from).collect())
                        .collect(),
                )
                .unwrap();
                let warm = planner
                    .warm_start
                    .as_ref()
                    .expect("a valid forecast must save its next plan");
                assert_eq!(warm.observation.day, game.day);
                assert_eq!(warm.observation.agents, game.agents);
                assert_eq!(warm.observation.traffics, game.traffic());
            }

            #[test]
            fn racing_pool_keeps_the_selected_plan_and_next_observation_consistent() {
                check_pool_forecast(true, 0, false);
            }

            #[test]
            fn predictive_pool_forecasts_match_the_selected_plan_in_both_budget_modes() {
                check_pool_forecast(false, 1, false);
                check_pool_forecast(true, 1, false);
            }

            #[test]
            fn reserve_tiebreak_preserves_the_selected_plan_and_warm_start() {
                check_pool_forecast(false, 0, true);
                check_pool_forecast(true, 0, true);
            }

            #[test]
            fn reserve_keeps_patrols_mobile_only_when_days_remain_after_the_forecast() {
                use tester::model::{Map, Spot};
                let problem = Problem {
                    starts_at: 0,
                    day_seconds: vec![60; 3],
                    day_steps: vec![2; 3],
                    map: Map {
                        height: 1,
                        width: 5,
                        cells: vec![vec![0; 5]],
                    },
                    spots: vec![
                        Spot {
                            brand: 0,
                            pos: 1,
                            stocks: 1,
                        },
                        Spot {
                            brand: 1,
                            pos: 3,
                            stocks: 1,
                        },
                    ],
                    agents: vec![0, 4],
                    fuel_limits: 4,
                    players: 1,
                    busy_threshold: 1,
                    jammed_threshold: 3,
                };
                let mut stranded = tester::engine::Game::new(problem.clone(), &[0, 0]);
                stranded.day = 2;
                stranded.agents[1].fuel = 0;
                let mut balanced = stranded.clone();
                balanced.agents[0].fuel = 2;
                balanced.agents[1].fuel = 2;
                let mut planner = Planner::new(problem, 0);
                planner.config.exact_pool_reserve = true;
                assert!(
                    planner.forecast_reserve(&balanced.agents, 1)
                        > planner.forecast_reserve(&stranded.agents, 1)
                );
                assert_eq!(planner.forecast_reserve(&balanced.agents, 2), (0, 0, 0));
                assert_eq!(planner.forecast_reserve(&stranded.agents, 2), (0, 0, 0));
                planner.config.exact_pool_reserve = false;
                assert_eq!(planner.forecast_reserve(&balanced.agents, 1), (0, 0, 0));
                assert_eq!(planner.forecast_reserve(&stranded.agents, 1), (0, 0, 0));
                // Both states have four units of fuel in total. Only the balanced state
                // can collect both brands on the following day in the independent game.
                stranded.play_day(vec![vec![2], vec![-2]]).unwrap();
                balanced.play_day(vec![vec![2], vec![5]]).unwrap();
                assert_eq!(stranded.score.daily_brands, 1);
                assert_eq!(balanced.score.daily_brands, 2);
            }

            #[test]
            fn relaxed_pool_keeps_one_day_tradeoffs_without_losing_new_brands() {
                let problem = tester::generator::generate(0, None).unwrap().problem;
                let planner = Planner::new(problem, 0);
                let mut pool = Pool {
                    active: true,
                    entries: vec![],
                    reference: None,
                    cap: 12,
                    diverse: true,
                    daily_slack: 1,
                };
                let candidate = |daily: u32, newkinds: u32| Sim {
                    claimed: (1_u32 << daily) - 1,
                    newkinds,
                    ..Sim::default()
                };
                pool.add(&planner, &candidate(8, 0), &vec![vec![0]]);
                pool.add(&planner, &candidate(9, 0), &vec![vec![1]]);
                assert_eq!(pool.entries.len(), 2);
                pool.add(&planner, &candidate(7, 0), &vec![vec![2]]);
                assert_eq!(pool.entries.len(), 2);
                pool.add(&planner, &candidate(10, 0), &vec![vec![3]]);
                assert_eq!(pool.entries.len(), 2);
                assert!(pool.entries.iter().all(|e| e.sim.claimed.count_ones() >= 9));
                pool.add(&planner, &candidate(8, 1), &vec![vec![4]]);
                assert_eq!(pool.entries.len(), 1);
                pool.add(&planner, &candidate(10, 0), &vec![vec![5]]);
                assert_eq!(pool.entries.len(), 1);
                assert_eq!(pool.entries[0].sim.newkinds, 1);
            }
        }
    }

    // 元ファイル: solver/assignment.rs
    mod assignment {
        use super::*;
        use tester::model::{Agent, Traffic};

        #[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
        struct Score {
            total: u32,
            daily: u32,
            balls: i32,
            patrols: usize,
        }

        impl Planner {
            fn assignment_dij(&mut self, pos: usize) -> Arc<Dij> {
                self.assignment_distances[pos]
                    .get_or_insert_with(|| {
                        Arc::new(Dij::new(pos, &self.map, &vec![0; self.map.cells.len()]))
                    })
                    .clone()
            }
            fn evaluate_assignment(&mut self, kinds: &[u8]) -> Score {
                let mut pos = self.map.problem.agents.clone();
                let n = pos.len();
                let cap = self.map.problem.fuel_limits as i32;
                let mut fuel = vec![cap; n];
                let mut ever = 0_u32;
                let patrols: Vec<_> = (0..n).filter(|&p| kinds[p] == 0).collect();
                let supplies: Vec<_> = (0..n).filter(|&p| kinds[p] == 1).collect();
                let mut score = Score {
                    patrols: patrols.len(),
                    ..Score::default()
                };
                for day in 0..self.map.problem.day_steps.len() {
                    let steps = self.map.problem.day_steps[day] as i32;
                    let mut stock: Vec<_> = self
                        .map
                        .problem
                        .spots
                        .iter()
                        .map(|s| s.stocks as i32)
                        .collect();
                    let mut claimed = 0;
                    let mut order = patrols.clone();
                    let mut supply_of = vec![None; n];
                    let mut escorted = vec![false; n];
                    let mut supply_dij = vec![];
                    let mut taken = vec![false; n];
                    for (si, &s) in supplies.iter().enumerate() {
                        let d = self.assignment_dij(pos[s]);
                        let mut best = None;
                        let mut best_dist = INF;
                        for &p in &patrols {
                            if taken[p] {
                                continue;
                            }
                            let distance = d.dist[pos[p]];
                            if best.is_none_or(|b| {
                                fuel[p] < fuel[b] || (fuel[p] == fuel[b] && distance < best_dist)
                            }) {
                                best = Some(p);
                                best_dist = distance;
                            }
                        }
                        if let Some(p) = best {
                            taken[p] = true;
                            supply_of[p] = Some(si);
                            escorted[p] = pos[s] == pos[p];
                        }
                        supply_dij.push(d);
                    }
                    order.sort_by_key(|&p| std::cmp::Reverse(fuel[p]));
                    order.sort_by_key(|&p| !escorted[p]);
                    for p in order {
                        let mut used = 0;
                        let mut escort = escorted[p];
                        let mut rendezvoused = false;
                        let mut visited = 0;
                        if steps > 0 && self.collect(pos[p], &mut stock, &mut visited, &mut claimed)
                        {
                            score.balls += 1;
                        }
                        ever |= claimed;
                        while used < steps {
                            let d = self.assignment_dij(pos[p]);
                            let mut best: Option<(usize, i32, i32, i32)> = None;
                            let mut advance = None;
                            let (mut advance_tier, mut advance_dist) = (-1, INF);
                            for (i, s) in self.map.problem.spots.iter().enumerate() {
                                if stock[i] <= 0
                                    || visited & (1 << i) != 0
                                    || s.pos == pos[p]
                                    || d.dist[s.pos] >= INF
                                {
                                    continue;
                                }
                                let bit = 1 << s.brand;
                                let tier = if ever & bit == 0 {
                                    3
                                } else if claimed & bit == 0 {
                                    1
                                } else {
                                    0
                                };
                                let distance = d.dist[s.pos];
                                if distance <= steps - used
                                    && best.is_none_or(|b| {
                                        (tier, -distance, stock[i]) > (b.1, -b.2, b.3)
                                    })
                                {
                                    best = Some((s.pos, tier, distance, stock[i]));
                                }
                                if tier >= 1
                                    && (tier > advance_tier
                                        || (tier == advance_tier && distance < advance_dist))
                                {
                                    advance = Some(s.pos);
                                    advance_tier = tier;
                                    advance_dist = distance;
                                }
                            }
                            let Some(target) = best.map(|b| b.0).or(advance) else {
                                break;
                            };
                            let dirs = d.path(pos[p], target);
                            if dirs.is_empty() {
                                break;
                            }
                            let mut progressed = false;
                            for dir in dirs {
                                let cell = pos[p];
                                let cost = match self.map.cells[cell] {
                                    0 => 2,
                                    1 => 1,
                                    _ => 3,
                                };
                                let fc = self.map.fuel_cost(cell);
                                if !escort
                                    && fuel[p] < fc
                                    && !rendezvoused
                                    && let Some(si) = supply_of[p]
                                {
                                    let resume = used.max(supply_dij[si].dist[cell]);
                                    if resume <= steps {
                                        used = resume;
                                        fuel[p] = cap;
                                        escort = true;
                                        rendezvoused = true;
                                    }
                                }
                                if used + cost > steps || (!escort && fuel[p] < fc) {
                                    break;
                                }
                                used += cost;
                                if !escort {
                                    fuel[p] -= fc;
                                }
                                pos[p] = self.map.neighbors[cell][dir as usize].unwrap();
                                progressed = true;
                                if self.collect(pos[p], &mut stock, &mut visited, &mut claimed) {
                                    score.balls += 1;
                                }
                                ever |= claimed;
                            }
                            if !progressed {
                                break;
                            }
                        }
                        if let Some(si) = supply_of[p]
                            && (escort || supply_dij[si].dist[pos[p]] <= steps)
                        {
                            fuel[p] = cap;
                            pos[supplies[si]] = pos[p];
                        }
                    }
                    score.daily += claimed.count_ones();
                }
                score.total = ever.count_ones();
                score
            }

            fn evaluate_planned(
                &mut self,
                kinds: &[u8],
                candidate_ms: u64,
                iterations: u64,
            ) -> Score {
                let saved_config = self.config.clone();
                let saved_kinds = self.kinds.clone();
                let saved_brands = self.brands_ever;
                let saved_rng = self.rng;
                self.kinds = kinds.to_vec();
                self.brands_ever = 0;
                self.config.pool = false;
                self.config.phasec_ms = 0;
                self.config.force_session = false;
                self.config.iterations = iterations;
                if iterations > 0 {
                    self.config.beam = false;
                }
                let days = self.map.problem.day_steps.len();
                self.config.budget_ms = if iterations > 0 {
                    1_000_000
                } else {
                    (candidate_ms / days.max(1) as u64).max(100)
                };
                let mut pos = self.map.problem.agents.clone();
                let mut fuel = vec![self.map.problem.fuel_limits as i32; pos.len()];
                let mut stays: Vec<Vec<i32>> = vec![];
                let mut ever = 0_u32;
                let mut score = Score {
                    patrols: kinds.iter().filter(|&&k| k == 0).count(),
                    ..Score::default()
                };
                for day in 0..days {
                    let mut traffics = vec![];
                    if day > 0 {
                        for (cell, &terrain) in self.map.cells.iter().enumerate() {
                            if terrain != 1 {
                                continue;
                            }
                            let volume = (stays[day - 1][cell]
                                + if day >= 2 { stays[day - 2][cell] } else { 0 })
                                / self.map.problem.players.max(1) as i32;
                            let status = if volume >= self.map.problem.jammed_threshold as i32 {
                                2
                            } else if volume >= self.map.problem.busy_threshold as i32 {
                                1
                            } else {
                                0
                            };
                            if status > 0 {
                                traffics.push(Traffic { pos: cell, status });
                            }
                        }
                    }
                    let observation = Observation {
                        ends_at: 0,
                        day,
                        agents: (0..pos.len())
                            .map(|p| Agent {
                                kind: kinds[p],
                                pos: pos[p],
                                fuel: fuel[p] as u32,
                            })
                            .collect(),
                        others: vec![],
                        traffics,
                    };
                    let ops =
                        self.plan_day_local(&observation, self.map.problem.day_steps[day] as i32);
                    if let Some(actual) = self.replay(&ops, &pos, &fuel) {
                        score.daily += actual.claimed.count_ones();
                        score.balls += actual.balls;
                        ever |= actual.claimed;
                        pos = actual.pos;
                        fuel = actual.fuel;
                        stays.push(actual.stay);
                    } else {
                        break;
                    }
                }
                score.total = ever.count_ones();
                self.config = saved_config;
                self.kinds = saved_kinds;
                self.brands_ever = saved_brands;
                self.rng = saved_rng;
                score
            }

            pub fn decide_kinds(&mut self) -> Vec<u8> {
                if config::setting("COVER_BEAM", 0) != 0
                    && self.map.brands <= 16
                    && self.map.problem.agents.len() == 4
                {
                    return self.cover_beam_select();
                }
                if config::setting("HORIZON", 0) != 0 {
                    return self.horizon_select();
                }
                let start = Instant::now();
                let n = self.map.problem.agents.len();
                self.kinds = vec![0; n];
                let mask = config::setting("SUPPLY_MASK", -1_i64);
                if mask >= 0 && mask < (1_i64 << n) - 1 {
                    self.kinds = (0..n).map(|p| ((mask >> p) & 1) as u8).collect();
                    return self.kinds.clone();
                }
                let forced = config::setting("NUM_SUPPLY", -1_i32);
                if forced >= 0 {
                    for k in self
                        .kinds
                        .iter_mut()
                        .rev()
                        .take((forced as usize).min(n.saturating_sub(1)))
                    {
                        *k = 1;
                    }
                    return self.kinds.clone();
                }
                let mut candidates = vec![vec![0; n]];
                for p in 0..n {
                    let mut c = vec![0; n];
                    c[p] = 1;
                    candidates.push(c);
                }
                let max_supply = if n >= 7 {
                    3
                } else if n >= 5 {
                    2
                } else {
                    1
                };
                for count in 2..=max_supply {
                    let mut group: Vec<Vec<u8>> = (0_u32..1 << n)
                        .filter(|m| m.count_ones() as usize == count)
                        .map(|m| (0..n).map(|p| ((m >> p) & 1) as u8).collect())
                        .collect();
                    group.sort();
                    candidates.extend(group);
                }
                if config::setting("EXACT_INIT", 0) != 0 && self.map.brands <= 16 {
                    let mut feasible = vec![];
                    for c in &candidates {
                        let observation = Observation {
                            ends_at: 0,
                            day: 0,
                            agents: self
                                .map
                                .problem
                                .agents
                                .iter()
                                .enumerate()
                                .map(|(p, &pos)| Agent {
                                    kind: c[p],
                                    pos,
                                    fuel: self.map.problem.fuel_limits,
                                })
                                .collect(),
                            others: vec![],
                            traffics: vec![],
                        };
                        self.prepare(&observation, self.map.problem.day_steps[0] as i32);
                        if self.exact_partition().is_some() {
                            feasible.push(c.clone());
                        }
                    }
                    if !feasible.is_empty() {
                        candidates = feasible;
                    }
                }
                let scores: Vec<_> = candidates
                    .iter()
                    .map(|c| self.evaluate_assignment(c))
                    .collect();
                let mut order: Vec<_> = (0..candidates.len()).collect();
                order.sort_unstable_by(|&a, &b| scores[b].cmp(&scores[a]));
                if self.config.debug > 0 {
                    for &i in &order {
                        eprintln!("[assign] kinds={:?} score={:?}", candidates[i], scores[i]);
                    }
                }
                let final_n = order.len().min(self.config.assign_k);
                // Keep fixed-iteration comparisons unchanged. With a time budget, use
                // half for screening and reserve the rest for the strongest fleets.
                let racing = self.config.assign_race && self.config.assign_iters == 0;
                let screening_ms = if racing {
                    self.config.assign_ms.saturating_sub(1000) / 2
                } else {
                    self.config.assign_ms.saturating_sub(1000)
                };
                let mut planned = Vec::with_capacity(final_n);
                let mut best = None;
                let mut chosen = 0;
                for f in 0..final_n {
                    if !self.config.assign_plan || self.expired() {
                        break;
                    }
                    let candidate_ms = if self.config.assign_iters > 0 {
                        0
                    } else {
                        let remaining = screening_ms.saturating_sub(Self::elapsed(start));
                        let ms = remaining / (final_n - f) as u64;
                        if ms < 400 {
                            break;
                        }
                        ms
                    };
                    let score = self.evaluate_planned(
                        &candidates[order[f]],
                        candidate_ms,
                        self.config.assign_iters,
                    );
                    planned.push((f, score));
                    if best.is_none_or(|b| score > b) {
                        best = Some(score);
                        chosen = f;
                    }
                    if self.config.debug > 0 {
                        eprintln!(
                            "[assign planned] kinds={:?} score={score:?}",
                            candidates[order[f]]
                        );
                    }
                }
                if racing && planned.len() > 1 {
                    planned.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                    planned.truncate(self.config.assign_finalists);
                    for (rank, &(f, _)) in planned.iter().enumerate() {
                        if self.expired() {
                            break;
                        }
                        let remaining = self
                            .config
                            .assign_ms
                            .saturating_sub(Self::elapsed(start) + 1000);
                        let candidate_ms = remaining / (planned.len() - rank) as u64;
                        if candidate_ms < 400 {
                            break;
                        }
                        let score = self.evaluate_planned(&candidates[order[f]], candidate_ms, 0);
                        // A longer rehearsal can take a different route. Retain the
                        // best score seen in either pass instead of erasing it.
                        if best.is_none_or(|b| score > b) {
                            best = Some(score);
                            chosen = f;
                        }
                        if self.config.debug > 0 {
                            eprintln!(
                                "[assign refined] kinds={:?} score={score:?}",
                                candidates[order[f]]
                            );
                        }
                    }
                }
                self.kinds = candidates[order[chosen]].clone();
                if self.config.debug > 0 {
                    eprintln!(
                        "[assign chosen] {:?} elapsed={}ms",
                        self.kinds,
                        Self::elapsed(start)
                    );
                }
                self.kinds.clone()
            }
        }
    }

    // 元ファイル: solver/beam.rs
    mod beam {
        use super::*;
        use std::collections::HashSet;

        pub(super) struct Zobrist {
            pos: Vec<u64>,
            stock: [u64; 288],
            visited: [u64; 256],
            done: [u64; 8],
        }
        impl Zobrist {
            pub fn new(cells: usize) -> Self {
                let mut seed = 0x37c0de9e3779b9_u64;
                let mut next = || {
                    seed = seed.wrapping_add(0x9e3779b97f4a7c15);
                    let mut z = seed;
                    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
                    z ^ (z >> 31)
                };
                Self {
                    pos: (0..8 * cells).map(|_| next()).collect(),
                    stock: std::array::from_fn(|_| next()),
                    visited: std::array::from_fn(|_| next()),
                    done: std::array::from_fn(|_| next()),
                }
            }
        }

        #[derive(Clone, Copy, Default)]
        struct Node {
            pos: [u16; 8],
            fuel: [i16; 8],
            used: [i16; 8],
            pot: [f32; 8],
            visited: [u32; 8],
            stock: [u8; 32],
            claimed: u32,
            stock_mask: u32,
            done: u8,
            escort: u8,
            rdv: u8,
            balls: i16,
            hist: i32,
            hash: u64,
            eval: f64,
        }
        struct History {
            parent: i32,
            patrol: u8,
            spot: u8,
        }

        impl Planner {
            fn beam_collect(&self, node: &mut Node, pi: usize, cell: usize) -> Option<usize> {
                let si = self.map.spot_at[cell]?;
                if node.visited[pi] & (1 << si) != 0 {
                    return None;
                }
                node.visited[pi] |= 1 << si;
                node.hash ^= self.zobrist.visited[pi * 32 + si];
                if node.stock[si] == 0 {
                    return None;
                }
                node.hash ^= self.zobrist.stock[si * 9 + node.stock[si] as usize];
                node.stock[si] -= 1;
                node.hash ^= self.zobrist.stock[si * 9 + node.stock[si] as usize];
                if node.stock[si] == 0 {
                    node.stock_mask &= !(1 << si);
                }
                node.claimed |= 1 << self.map.problem.spots[si].brand;
                node.balls += 1;
                Some(si)
            }
            fn beam_pot(&self, node: &Node, pi: usize) -> f32 {
                if node.done & (1 << pi) != 0 {
                    return 0.0;
                }
                let pos = node.pos[pi] as usize;
                let (mut best_value, mut best_any) = (INF, INF);
                let mut mask = node.stock_mask & !node.visited[pi];
                while mask != 0 {
                    let si = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    let spot = &self.map.problem.spots[si];
                    let distance = if self.precomputed[pos] {
                        self.distance(pos, spot.pos)
                    } else {
                        self.distance(spot.pos, pos)
                    };
                    if distance >= INF {
                        continue;
                    }
                    if node.claimed & (1 << spot.brand) == 0
                        || self.brands_ever & (1 << spot.brand) == 0
                    {
                        best_value = best_value.min(distance);
                    }
                    best_any = best_any.min(distance);
                }
                let d = if best_value < INF {
                    best_value
                } else {
                    best_any
                };
                if d < INF { d as f32 } else { 0.0 }
            }
            fn beam_eval(&self, n: &Node) -> f64 {
                let np = self.order.len();
                let pot: f64 = n.pot[..np].iter().map(|&v| v as f64).sum();
                let used: f64 = n.used[..np].iter().map(|&v| v as f64).sum();
                let fuel: f64 = n.fuel[..np]
                    .iter()
                    .map(|&v| (v as i32).min(self.map.problem.fuel_limits as i32) as f64)
                    .sum();
                (n.claimed & !self.brands_ever).count_ones() as f64 * 1e11
                    + n.claimed.count_ones() as f64 * 1e8
                    + n.balls as f64 * 1e4
                    - self.config.beam_pot * pot
                    - 0.1 * used
                    + 1e-4 * fuel
            }
            fn beam_done(&self, n: &mut Node, pi: usize) {
                n.done |= 1 << pi;
                n.hash ^= self.zobrist.done[pi];
                n.pot[pi] = 0.0;
            }
            fn beam_walk(&self, n: &mut Node, pi: usize, target: usize) {
                let (mut pos, mut fuel, mut used) =
                    (n.pos[pi] as usize, n.fuel[pi] as i32, n.used[pi] as i32);
                let start = pos;
                let mut escort = n.escort & (1 << pi) != 0;
                let mut rdv = n.rdv & (1 << pi) != 0;
                if !self.precomputed[pos] {
                    self.beam_done(n, pi);
                    return;
                }
                let dirs = self.distances[pos]
                    .as_ref()
                    .unwrap()
                    .path_view(pos, self.map.problem.spots[target].pos);
                if dirs.is_empty() {
                    self.beam_done(n, pi);
                    return;
                }
                for &dir in dirs.iter() {
                    let (cost, fc) = (self.step_cost[pos], self.fuel_cost[pos]);
                    if !escort
                        && fuel < fc
                        && !rdv
                        && let Some(resume) = self.rendezvous(self.order[pi], pos, used)
                    {
                        used = resume;
                        fuel = self.map.problem.fuel_limits as i32;
                        escort = true;
                        rdv = true;
                    }
                    if used + cost > self.day_steps || (!escort && fuel < fc) {
                        self.beam_done(n, pi);
                        break;
                    }
                    used += cost;
                    if !escort {
                        fuel -= fc;
                    }
                    pos = self.map.neighbors[pos][dir as usize].unwrap();
                    self.beam_collect(n, pi, pos);
                }
                let nc = self.map.cells.len();
                if pos != start {
                    n.hash ^= self.zobrist.pos[pi * nc + start] ^ self.zobrist.pos[pi * nc + pos];
                }
                n.pos[pi] = pos as u16;
                n.fuel[pi] = fuel as i16;
                n.used[pi] = used as i16;
                if escort {
                    n.escort |= 1 << pi;
                }
                if rdv {
                    n.rdv |= 1 << pi;
                }
            }
            /// None は移動不能。Some(None) は収集を伴わない移動。
            fn beam_step(&self, n: &mut Node, pi: usize, dir: usize) -> Option<Option<usize>> {
                let (pos, mut fuel, mut used) =
                    (n.pos[pi] as usize, n.fuel[pi] as i32, n.used[pi] as i32);
                let mut escort = n.escort & (1 << pi) != 0;
                let mut rdv = n.rdv & (1 << pi) != 0;
                let (cost, fc) = (self.step_cost[pos], self.fuel_cost[pos]);
                if !escort
                    && fuel < fc
                    && !rdv
                    && let Some(resume) = self.rendezvous(self.order[pi], pos, used)
                {
                    used = resume;
                    fuel = self.map.problem.fuel_limits as i32;
                    escort = true;
                    rdv = true;
                }
                if used + cost > self.day_steps || (!escort && fuel < fc) {
                    return None;
                }
                let next = self.map.neighbors[pos][dir]?;
                used += cost;
                if !escort {
                    fuel -= fc;
                }
                let nc = self.map.cells.len();
                n.hash ^= self.zobrist.pos[pi * nc + pos] ^ self.zobrist.pos[pi * nc + next];
                n.pos[pi] = next as u16;
                n.fuel[pi] = fuel as i16;
                n.used[pi] = used as i16;
                if escort {
                    n.escort |= 1 << pi;
                }
                if rdv {
                    n.rdv |= 1 << pi;
                }
                Some(self.beam_collect(n, pi, next))
            }
            fn beam_candidates(&self, n: &Node, pi: usize) -> Vec<usize> {
                let pos = n.pos[pi] as usize;
                let remaining = self.day_steps - n.used[pi] as i32;
                if remaining <= 0 || !self.precomputed[pos] {
                    return vec![];
                }
                let d = self.distances[pos].as_ref().unwrap();
                let mut candidates = vec![];
                let (mut advance, mut adv_tier, mut adv_dist) = (None, -1, INF);
                let mut mask = n.stock_mask & !n.visited[pi];
                while mask != 0 {
                    let si = mask.trailing_zeros() as usize;
                    mask &= mask - 1;
                    let spot = &self.map.problem.spots[si];
                    let distance = d.dist[spot.pos];
                    if spot.pos == pos || distance >= INF {
                        continue;
                    }
                    let bit = 1 << spot.brand;
                    let tier = if self.brands_ever & bit == 0 && n.claimed & bit == 0 {
                        3
                    } else if n.claimed & bit == 0 {
                        1
                    } else {
                        0
                    };
                    if distance <= remaining {
                        candidates.push((si, tier, distance, n.stock[si]));
                    }
                    if tier >= 1 && (tier > adv_tier || (tier == adv_tier && distance < adv_dist)) {
                        advance = Some(si);
                        adv_tier = tier;
                        adv_dist = distance;
                    }
                }
                if candidates.is_empty() {
                    return advance.into_iter().collect();
                }
                candidates
                    .sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)).then(b.3.cmp(&a.3)));
                candidates
                    .iter()
                    .take(self.config.beam_k)
                    .map(|c| c.0)
                    .collect()
            }
            fn beam_expand_steps(
                &self,
                n: Node,
                movers: &[usize],
                next: &mut Vec<Node>,
                hist: &mut Vec<History>,
                cap: usize,
            ) {
                if next.len() >= cap {
                    return;
                }
                let Some((&pi, rest)) = movers.split_first() else {
                    let mut child = n;
                    child.eval = self.beam_eval(&child);
                    next.push(child);
                    return;
                };
                let mut moved = false;
                if n.stock_mask & !n.visited[pi] != 0 {
                    for dir in 0..6 {
                        let Some(pos) = self.map.neighbors[n.pos[pi] as usize][dir] else {
                            continue;
                        };
                        if self.map.cells[pos] == 3 {
                            continue;
                        }
                        let mut child = n;
                        let Some(got) = self.beam_step(&mut child, pi, dir) else {
                            break;
                        };
                        moved = true;
                        if let Some(spot) = got {
                            hist.push(History {
                                parent: child.hist,
                                patrol: pi as u8,
                                spot: spot as u8,
                            });
                            child.hist = hist.len() as i32 - 1;
                        }
                        child.pot[pi] = self.beam_pot(&child, pi);
                        self.beam_expand_steps(child, rest, next, hist, cap);
                    }
                }
                if !moved {
                    let mut child = n;
                    self.beam_done(&mut child, pi);
                    self.beam_expand_steps(child, rest, next, hist, cap);
                }
            }
            fn beam_run(
                &self,
                width: usize,
                start: Instant,
                deadline: u64,
                mode: usize,
            ) -> (Tours, bool) {
                let np = self.order.len();
                let nc = self.map.cells.len();
                let ns = self.map.problem.spots.len();
                let mut root = Node {
                    hist: -1,
                    ..Node::default()
                };
                for (pi, &p) in self.order.iter().enumerate() {
                    root.pos[pi] = self.apos[p] as u16;
                    root.fuel[pi] = self.afuel[p] as i16;
                    if self.optimistic[p] {
                        root.escort |= 1 << pi;
                    }
                    root.hash ^= self.zobrist.pos[pi * nc + self.apos[p]];
                }
                for (i, s) in self.map.problem.spots.iter().enumerate() {
                    root.stock[i] = s.stocks as u8;
                    if s.stocks > 0 {
                        root.stock_mask |= 1 << i;
                    }
                    root.hash ^= self.zobrist.stock[i * 9 + s.stocks as usize];
                }
                if self.day_steps > 0 {
                    for pi in 0..np {
                        let pos = root.pos[pi] as usize;
                        self.beam_collect(&mut root, pi, pos);
                    }
                }
                for pi in 0..np {
                    root.pot[pi] = self.beam_pot(&root, pi);
                }
                root.eval = self.beam_eval(&root);
                let (mut current, mut next) = (vec![root], vec![]);
                let mut hist = Vec::with_capacity((width * self.config.beam_k * 8).min(1 << 22));
                let mut best = root;
                let mut terminal: Option<Node> = None;
                let mut saturated = false;
                let max_depth =
                    np * (if mode == 0 {
                        ns + 1
                    } else {
                        self.day_steps as usize + 1
                    }) + 1;
                let cap = if mode == 3 {
                    1 << 21
                } else {
                    self.config.beam_children_cap
                };
                let mut seen = HashSet::new();
                for _ in 0..max_depth {
                    if current.is_empty() || Self::elapsed(start) >= deadline || self.expired() {
                        break;
                    }
                    next.clear();
                    let mut timeout = false;
                    for node in &current {
                        if (mode == 3 || next.len() & 1023 == 0)
                            && (Self::elapsed(start) >= deadline || self.expired())
                        {
                            timeout = true;
                            break;
                        }
                        if next.len() + 32 > cap {
                            break;
                        }
                        let pi = (0..np)
                            .filter(|&pi| node.done & (1 << pi) == 0)
                            .min_by_key(|&pi| node.used[pi]);
                        let Some(pi) = pi else {
                            if terminal.is_none_or(|t| node.eval > t.eval) {
                                terminal = Some(*node);
                            }
                            continue;
                        };
                        if mode != 0 {
                            let movers = if mode == 3 {
                                (0..np)
                                    .filter(|&i| {
                                        node.done & (1 << i) == 0 && node.used[i] == node.used[pi]
                                    })
                                    .collect()
                            } else {
                                vec![pi]
                            };
                            let begin = next.len();
                            self.beam_expand_steps(*node, &movers, &mut next, &mut hist, cap);
                            if mode == 3 {
                                for child in &mut next[begin..] {
                                    for &i in &movers {
                                        child.pot[i] = self.beam_pot(child, i);
                                    }
                                    child.eval = self.beam_eval(child);
                                }
                            }
                            continue;
                        }
                        let candidates = self.beam_candidates(node, pi);
                        if candidates.is_empty() {
                            let mut child = *node;
                            self.beam_done(&mut child, pi);
                            child.eval = self.beam_eval(&child);
                            next.push(child);
                        }
                        for spot in candidates {
                            let mut child = *node;
                            self.beam_walk(&mut child, pi, spot);
                            hist.push(History {
                                parent: node.hist,
                                patrol: pi as u8,
                                spot: spot as u8,
                            });
                            child.hist = hist.len() as i32 - 1;
                            child.pot[pi] = self.beam_pot(&child, pi);
                            child.eval = self.beam_eval(&child);
                            next.push(child);
                        }
                    }
                    if next.is_empty() {
                        break;
                    }
                    let mut indices: Vec<_> = (0..next.len()).collect();
                    let cmp = |&a: &usize, &b: &usize| {
                        next[b]
                            .eval
                            .total_cmp(&next[a].eval)
                            .then(next[a].hash.cmp(&next[b].hash))
                    };
                    if mode == 3 && indices.len() > width * 4 {
                        indices.select_nth_unstable_by(width * 4, cmp);
                        indices.truncate(width * 4);
                    }
                    indices.sort_unstable_by(cmp);
                    current.clear();
                    seen.clear();
                    for i in indices {
                        if seen.insert(next[i].hash) {
                            current.push(next[i]);
                            if current.len() >= width {
                                saturated = true;
                                break;
                            }
                        }
                    }
                    if !current.is_empty() && current[0].eval > best.eval {
                        best = current[0];
                    }
                    if timeout {
                        break;
                    }
                }
                let chosen = terminal.unwrap_or(best);
                let mut tours = vec![vec![]; self.na];
                let mut h = chosen.hist;
                while h >= 0 {
                    let item = &hist[h as usize];
                    tours[self.order[item.patrol as usize]].push(item.spot as usize);
                    h = item.parent;
                }
                for &p in &self.patrols {
                    tours[p].reverse();
                }
                (tours, saturated)
            }
            pub(super) fn beam_init(&mut self, start: Instant, deadline: u64, tours: &mut Tours) {
                if self.patrols.is_empty() || self.map.problem.spots.is_empty() {
                    return;
                }
                let mut best = self.simulate(tours);
                self.rollout(&mut best);
                let mut beam_seen = false;
                let mut phases = vec![(self.config.beam_step, deadline, false)];
                match self.config.beam_step {
                    2 => {
                        let now = Self::elapsed(start);
                        phases = vec![
                            (0, now + deadline.saturating_sub(now) / 2, false),
                            (1, deadline, false),
                        ];
                    }
                    3 => phases = vec![(3, deadline, true)],
                    4 => {
                        phases = vec![
                            (0, deadline.min(Self::elapsed(start) + 2500), false),
                            (3, deadline, true),
                        ]
                    }
                    5 => phases = vec![(0, deadline, true)],
                    _ => {}
                }
                for (mode, until, fixed) in phases {
                    let mut width = if fixed && self.config.beam_width == 128 {
                        1 << 16
                    } else {
                        self.config.beam_width
                    };
                    loop {
                        if self.expired() {
                            break;
                        }
                        let (cand, saturated) = self.beam_run(width, start, until, mode);
                        let mut s = self.simulate(&cand);
                        self.rollout(&mut s);
                        if (!fixed && self.config.init_mode == 2 && !beam_seen)
                            || self.better(&s, &best)
                        {
                            *tours = cand;
                            best = s;
                        }
                        beam_seen = true;
                        if self.config.debug == 2 {
                            eprintln!(
                                "[day {} beam] width={width} saturated={saturated} elapsed={}ms",
                                self.day_index,
                                Self::elapsed(start)
                            );
                        }
                        if fixed
                            || !saturated
                            || Self::elapsed(start) >= until
                            || width >= self.config.beam_width_cap
                        {
                            break;
                        }
                        width = width.saturating_mul(2);
                        if width > self.config.beam_width_cap {
                            break;
                        }
                    }
                }
            }
        }
    }

    // 元ファイル: solver/config.rs
    pub mod config {
        use std::{env, str::FromStr};

        pub fn setting<T: FromStr>(name: &str, default: T) -> T {
            env::var(format!("PROCON37_{name}"))
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(default)
        }
        fn flag(name: &str, default: bool) -> bool {
            setting(name, i32::from(default)) != 0
        }

        #[derive(Clone)]
        pub struct Config {
            pub budget_ms: u64,
            pub iterations: u64,
            pub depth: usize,
            pub future_balls: bool,
            pub future_nk: bool,
            pub stack_targets: bool,
            pub enumerate_supply_targets: bool,
            pub rollout_escorts: bool,
            pub oropt: bool,
            pub pool: bool,
            pub pool_cap: usize,
            pub exact_pool: bool,
            pub exact_pool_search_depth: usize,
            pub exact_pool_reserve: bool,
            pub pool_diverse: bool,
            pub memo: bool,
            pub lazy: bool,
            pub retry: usize,
            pub retry_mode: usize,
            pub reachable_coverage: bool,
            pub restart_prob: f64,
            pub mutate_prob: f64,
            pub restart_mode: usize,
            pub session_mode: usize,
            pub session_ms: u64,
            pub force_session: bool,
            pub final_continue: bool,
            pub supply_mode: usize,
            pub exact_claimed: bool,
            pub beam: bool,
            pub beam_ms: Option<u64>,
            pub beam_k: usize,
            pub beam_width: usize,
            pub beam_width_cap: usize,
            pub beam_children_cap: usize,
            pub beam_pot: f64,
            pub beam_step: usize,
            pub init_mode: usize,
            pub ins_mode: usize,
            pub neighbor_mode: usize,
            pub reach: bool,
            pub verify_depth: usize,
            pub verify_agg: usize,
            pub atb: bool,
            pub phasec_ms: u64,
            pub sa: bool,
            pub sa_t0: f64,
            pub sa_t1: f64,
            pub population: bool,
            pub pop_width: usize,
            pub pop_elite: usize,
            pub pop_cull_iters: u64,
            pub pop_cull_ms: u64,
            pub pop_branch: usize,
            pub assign_ms: u64,
            pub assign_k: usize,
            pub assign_plan: bool,
            pub assign_iters: u64,
            pub assign_race: bool,
            pub assign_finalists: usize,
            pub debug: usize,
        }

        impl Config {
            pub fn from_env() -> Self {
                let budget_ms = setting("TIME_MS", 55_000);
                let beam_ms = setting("BEAM_MS", -1_i64);
                let exact_pool = flag("EXACT_POOL", true);
                Self {
                    budget_ms,
                    iterations: setting("ITERS", 0),
                    depth: setting("DEPTH", if budget_ms >= 20_000 { 2 } else { 1 }),
                    future_balls: flag("FUTURE_BALLS", false),
                    future_nk: flag("FUTURE_NK", true),
                    stack_targets: flag("STACK_TARGETS", false),
                    enumerate_supply_targets: flag("SUPPLY_ENUM", false),
                    rollout_escorts: flag("ROLLOUT_ESCORTS", false),
                    oropt: flag("OROPT", false),
                    pool: flag("POOL", true),
                    pool_cap: setting("POOL_CAP", 12).max(2),
                    exact_pool,
                    exact_pool_search_depth: setting("EXACT_POOL_SEARCH_DEPTH", 0_usize).min(2),
                    exact_pool_reserve: flag("EXACT_POOL_RESERVE", false),
                    pool_diverse: flag("POOL_DIVERSE", exact_pool),
                    memo: flag("MEMO", true),
                    lazy: flag("LAZY", true),
                    retry: setting("RETRY", 8),
                    retry_mode: setting("RETRY_MODE", 0),
                    reachable_coverage: flag("COVERAGE", true),
                    restart_prob: setting("RESTART_P", 0.60),
                    mutate_prob: setting("MUTATE_P", 0.15),
                    restart_mode: setting("RESTART_MODE", 1),
                    session_mode: setting("SESSION_MODE", 1),
                    session_ms: setting("SESSION_MS", 5000).max(100),
                    force_session: env::var_os("PROCON37_SESSION_MS").is_some(),
                    final_continue: flag("FINAL_CONTINUE", true),
                    supply_mode: setting("SUPPLY", 2),
                    exact_claimed: flag("EXACT_CLAIMED", true),
                    beam: flag("BEAM", true),
                    beam_ms: (beam_ms >= 0).then_some(beam_ms as u64),
                    beam_k: setting("BEAM_K", 64).max(1),
                    beam_width: setting("BEAM_W", 128).max(1),
                    beam_width_cap: setting("BEAM_WCAP", 4096).max(1),
                    beam_children_cap: setting("BEAM_CCAP", 65536).max(33),
                    beam_pot: setting("BEAM_POT", 1.0),
                    beam_step: setting("BEAM_STEP", 0),
                    init_mode: setting("INIT", 0),
                    ins_mode: setting("INS", 0),
                    neighbor_mode: setting("NEIGHBOR", 0),
                    reach: flag("REACH", false),
                    verify_depth: setting("VERIFY_DEPTH", 1).max(1),
                    verify_agg: setting("VERIFY_AGG", 0),
                    atb: flag("ATB", false),
                    phasec_ms: setting("PHASEC", 0),
                    sa: flag("SA", false),
                    sa_t0: setting("SA_T0", 4.0),
                    sa_t1: setting("SA_T1", 0.3),
                    population: flag("POP", false),
                    pop_width: setting("POP_W", 0),
                    pop_elite: setting("POP_ELITE", 0),
                    pop_cull_iters: setting("POP_CULL_ITERS", 240).max(1),
                    pop_cull_ms: setting("POP_CULL_MS", 1200),
                    pop_branch: setting("POP_BRANCH", 2).max(1),
                    assign_ms: setting("ASSIGN_MS", 55_000),
                    assign_k: setting("ASSIGN_K", 16).max(1),
                    assign_plan: flag("ASSIGN_PLAN", true) && setting("ASSIGN_R", 1_i64) > 0,
                    assign_iters: setting("ASSIGN_PLAN_ITERS", 0),
                    assign_race: flag("ASSIGN_RACE", false),
                    assign_finalists: setting("ASSIGN_FINALISTS", 3).max(1),
                    debug: setting("DEBUG", 0),
                }
            }
        }

        #[derive(Clone, Copy)]
        pub struct Prng {
            pub state: u32,
        }
        impl Prng {
            pub fn new(seed: u32) -> Self {
                Self { state: seed }
            }
            pub fn next(&mut self) -> f64 {
                self.state = self.state.wrapping_add(0x6d2b79f5);
                let mut t = self.state;
                t = (t ^ (t >> 15)).wrapping_mul(t | 1);
                t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
                f64::from(t ^ (t >> 14)) / 4294967296.0
            }
            pub fn index(&mut self, n: usize) -> usize {
                ((self.next() * n as f64) as usize).min(n.saturating_sub(1))
            }
            pub fn weighted(&mut self, weights: &[f64]) -> usize {
                let x = self.next() * weights.iter().sum::<f64>();
                let mut sum = 0.0;
                for (i, &w) in weights.iter().enumerate() {
                    sum += w;
                    if x <= sum {
                        return i;
                    }
                }
                weights.len() - 1
            }
        }
    }

    // 元ファイル: solver/cover.rs
    mod cover {
        //! 種類集合ごとの最短巡回と、巡回車への集合分割。
        use super::*;

        struct CoverDp {
            cost: Vec<i32>,
            parent: Vec<u8>,
            best: Vec<i32>,
            last: Vec<usize>,
            width: usize,
        }
        impl CoverDp {
            fn route(&self, mask: usize, spots: &[tester::model::Spot]) -> Vec<usize> {
                if mask == 0 {
                    return vec![];
                }
                self.route_ending(mask, self.last[mask], spots)
            }
            fn route_ending(
                &self,
                mut mask: usize,
                mut last: usize,
                spots: &[tester::model::Spot],
            ) -> Vec<usize> {
                let mut route = vec![];
                while mask != 0 {
                    route.push(last);
                    let prev = self.parent[mask * self.width + last];
                    mask &= !(1 << spots[last].brand);
                    if prev == u8::MAX {
                        break;
                    }
                    last = prev as usize;
                }
                route.reverse();
                route
            }
        }

        impl Planner {
            /// 異なる担当集合と終点を残す。全種類を当日中に訪問できる候補だけを返す。
            pub(super) fn exact_partition_samples(&mut self, limit: usize) -> Vec<Tours> {
                if self.map.brands > 16 || self.patrols.len() != 3 {
                    return vec![];
                }
                let full = (1 << self.map.brands) - 1;
                let tables: Vec<_> = self
                    .patrols
                    .iter()
                    .map(|&p| self.cover_dp(p, full))
                    .collect();
                let mut partitions = Vec::new();
                for a in 0..=full {
                    if tables[0].best[a] > self.day_steps {
                        continue;
                    }
                    let rest = full ^ a;
                    let mut b = rest;
                    loop {
                        let c = rest ^ b;
                        if tables[1].best[b] <= self.day_steps
                            && tables[2].best[c] <= self.day_steps
                        {
                            partitions.push([a, b, c]);
                        }
                        if b == 0 {
                            break;
                        }
                        b = (b - 1) & rest;
                    }
                }
                let mut result = Vec::new();
                if partitions.is_empty() {
                    return result;
                }
                let spots = &self.map.problem.spots;
                let mut unique = std::collections::HashSet::new();
                let mut alternatives = std::collections::HashMap::new();
                for _ in 0..limit * 12 {
                    let masks = partitions[self.rng.index(partitions.len())];
                    let mut tours = vec![vec![]; self.na];
                    for (i, &p) in self.patrols.iter().enumerate() {
                        if masks[i] == 0 {
                            continue;
                        }
                        if config::setting("COVER_BEAM_ORDERS", 1) != 0 {
                            let options = alternatives.entry((p, masks[i])).or_insert_with(|| {
                                let mut routes = Vec::new();
                                self.cover_orders(
                                    self.apos[p],
                                    masks[i],
                                    0,
                                    &mut Vec::new(),
                                    &mut routes,
                                );
                                routes
                            });
                            if !options.is_empty() {
                                tours[p] = options[self.rng.index(options.len())].clone();
                                continue;
                            }
                        }
                        let options: Vec<_> = (0..spots.len())
                            .filter(|&s| {
                                tables[i].cost[masks[i] * spots.len() + s] <= self.day_steps
                            })
                            .collect();
                        let end = options[self.rng.index(options.len())];
                        tours[p] = tables[i].route_ending(masks[i], end, spots);
                    }
                    if unique.insert(tours.clone()) {
                        result.push(tours);
                    }
                    if result.len() >= limit {
                        break;
                    }
                }
                result
            }

            fn cover_orders(
                &self,
                pos: usize,
                remaining: usize,
                elapsed: i32,
                route: &mut Vec<usize>,
                result: &mut Vec<Vec<usize>>,
            ) {
                if result.len() >= 4096 || self.expired() {
                    return;
                }
                if remaining == 0 {
                    result.push(route.clone());
                    return;
                }
                for (si, spot) in self.map.problem.spots.iter().enumerate() {
                    let bit = 1 << spot.brand;
                    if remaining & bit == 0 || spot.stocks == 0 {
                        continue;
                    }
                    let next = elapsed + self.distance(pos, spot.pos);
                    if next > self.day_steps {
                        continue;
                    }
                    route.push(si);
                    self.cover_orders(spot.pos, remaining ^ bit, next, route, result);
                    route.pop();
                }
            }

            fn cover_dp(&self, patrol: usize, allowed: usize) -> CoverDp {
                let n = self.map.problem.spots.len();
                let masks = 1 << self.map.brands;
                let mut result = CoverDp {
                    cost: vec![INF; masks * n],
                    parent: vec![u8::MAX; masks * n],
                    best: vec![INF; masks],
                    last: vec![0; masks],
                    width: n,
                };
                result.best[0] = 0;
                let spots = &self.map.problem.spots;
                for (i, s) in spots.iter().enumerate() {
                    let bit = 1 << s.brand;
                    if bit & allowed == 0 || s.stocks == 0 {
                        continue;
                    }
                    result.cost[bit * n + i] = self.distance(self.apos[patrol], s.pos);
                }
                for mask in 1..masks {
                    if mask & !allowed != 0 {
                        continue;
                    }
                    for (i, from) in spots.iter().enumerate() {
                        let time = result.cost[mask * n + i];
                        if time >= INF {
                            continue;
                        }
                        if time < result.best[mask] {
                            result.best[mask] = time;
                            result.last[mask] = i;
                        }
                        for (j, to) in spots.iter().enumerate() {
                            let bit = 1 << to.brand;
                            if bit & allowed == 0 || mask & bit != 0 || to.stocks == 0 {
                                continue;
                            }
                            let next = (mask | bit) * n + j;
                            let value = time + self.distance(from.pos, to.pos);
                            if value < result.cost[next] {
                                result.cost[next] = value;
                                result.parent[next] = i as u8;
                            }
                        }
                    }
                }
                result
            }

            pub(super) fn exact_partition(&self) -> Option<Tours> {
                if self.map.brands > 16 || self.map.brands == 0 {
                    return None;
                }
                let masks = 1 << self.map.brands;
                let tables: Vec<_> = self
                    .patrols
                    .iter()
                    .map(|&p| self.cover_dp(p, masks - 1))
                    .collect();
                let mut cost = vec![INF; masks];
                cost[0] = 0;
                let mut parents = vec![];
                for table in &tables {
                    let feasible: Vec<_> = (0..masks)
                        .filter(|&m| table.best[m] <= self.day_steps)
                        .collect();
                    let mut next = vec![INF; masks];
                    let mut parent = vec![usize::MAX; masks];
                    for mask in 0..masks {
                        if cost[mask] == INF {
                            continue;
                        }
                        for &part in &feasible {
                            if mask & part != 0 {
                                continue;
                            }
                            let value = cost[mask] + table.best[part];
                            if value < next[mask | part] {
                                next[mask | part] = value;
                                parent[mask | part] = part;
                            }
                        }
                    }
                    cost = next;
                    parents.push(parent);
                }
                if cost[masks - 1] >= INF {
                    return None;
                }
                let mut mask = masks - 1;
                let mut tours = vec![vec![]; self.na];
                for k in (0..tables.len()).rev() {
                    let part = parents[k][mask];
                    tours[self.patrols[k]] = tables[k].route(part, &self.map.problem.spots);
                    mask ^= part;
                }
                Some(tours)
            }

            pub(super) fn exact_group_routes(&self, groups: &[usize]) -> Tours {
                let mut tours = vec![vec![]; self.na];
                for &p in &self.patrols {
                    let table = self.cover_dp(p, groups[p]);
                    tours[p] = table.route(groups[p], &self.map.problem.spots);
                }
                tours
            }
        }

        impl Planner {
            pub(super) fn reorder_group(&mut self, patrol: usize, route: &[usize]) -> Vec<usize> {
                if self.map.brands > 16 || route.is_empty() {
                    return route.to_vec();
                }
                let mask = route
                    .iter()
                    .fold(0usize, |m, &si| m | (1 << self.map.problem.spots[si].brand));
                let table = self.cover_dp(patrol, mask);
                let mut endings: Vec<_> = (0..self.map.problem.spots.len())
                    .filter(|&i| table.cost[mask * table.width + i] < INF)
                    .collect();
                endings.sort_unstable_by_key(|&i| table.cost[mask * table.width + i]);
                if endings.is_empty() {
                    return route.to_vec();
                }
                let last = endings[self.rng.index(endings.len().min(4))];
                table.route_ending(mask, last, &self.map.problem.spots)
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use tester::{engine::Game, generator::generate};

            #[test]
            fn partition_covers_all_brands_with_individual_time_limits() {
                let problem = generate(0, None).unwrap().problem;
                for (kinds, possible) in [(vec![0, 0, 1, 0], true), (vec![0, 1, 0, 0], false)] {
                    let game = Game::new(problem.clone(), &kinds);
                    let mut planner = Planner::new(problem.clone(), 0);
                    planner.prepare(&game.observation(0), problem.day_steps[0] as i32);
                    let tours = planner.exact_partition();
                    assert_eq!(tours.is_some(), possible);
                    if let Some(tours) = tours {
                        let mut brands = 0;
                        for &p in &planner.patrols {
                            let mut pos = planner.apos[p];
                            let mut time = 0;
                            for &si in &tours[p] {
                                let s = &problem.spots[si];
                                time += planner.distance(pos, s.pos);
                                pos = s.pos;
                                brands |= 1 << s.brand;
                            }
                            assert!(time <= problem.day_steps[0] as i32);
                        }
                        assert_eq!(brands, (1 << planner.map.brands) - 1);
                    }
                }
            }
        }
    }

    // 元ファイル: solver/coverbeam.rs
    mod coverbeam {
        //! 全種類を獲得した日だけを残す日単位ビーム。固定した巡回経路の補給は時空間DPで解く。
        use super::*;
        use std::collections::HashSet;
        use tester::model::{Agent, Observation};

        #[derive(Clone)]
        pub(super) struct CoverSchedule {
            kinds: Vec<u8>,
            plans: Vec<Plan>,
            states: Vec<Observation>,
            agents: Vec<Agent>,
            history: Vec<Vec<i32>>,
            value: f64,
        }

        #[derive(Clone, Copy)]
        struct SupplyLabel {
            fuel: [i16; 8],
            pos: usize,
            time: usize,
            parent: usize,
            action: i32,
            road_cost: i32,
        }

        impl Planner {
            /// 巡回車の固定軌跡に対し、補給車1台の合法経路をラベル設定法で探索する。
            /// 同じ位置・時刻では燃料のパレート集合を残す。capによる打ち切りは探索上の制限。
            fn cover_supply(&self, patrol_plan: &Plan, per_cell: usize, keep: usize) -> Vec<Plan> {
                if self.supplies.len() != 1 {
                    return vec![];
                }
                let sup = self.supplies[0];
                let steps = self.day_steps as usize;
                let n = self.map.cells.len();
                let cap = self.map.problem.fuel_limits as i16;
                let mut positions = vec![vec![0; steps + 1]; self.na];
                let mut debit = vec![vec![0_i16; steps + 1]; self.na];
                for &p in &self.patrols {
                    let mut pos = self.apos[p];
                    let mut t = 0;
                    positions[p][0] = pos;
                    for &action in &patrol_plan[p] {
                        let duration = if action < 0 {
                            -action
                        } else {
                            self.step_cost[pos]
                        } as usize;
                        if t + duration > steps {
                            return vec![];
                        }
                        positions[p][t + 1..=t + duration].fill(pos);
                        if action >= 0 {
                            debit[p][t + duration] = self.fuel_cost[pos] as i16;
                            pos = self.map.neighbors[pos][action as usize].unwrap();
                            positions[p][t + duration] = pos;
                        }
                        t += duration;
                    }
                    if t != steps {
                        return vec![];
                    }
                }
                let mut road_counts = vec![0; n];
                for &p in &self.patrols {
                    for &pos in &positions[p][1..] {
                        road_counts[pos] += 1;
                    }
                }
                let road_weight = config::setting("COVER_BEAM_ROAD_COST", 0.2_f64);
                let mut fuel = [0; 8];
                for &p in &self.patrols {
                    fuel[p] = self.afuel[p] as i16;
                }
                let mut labels = vec![SupplyLabel {
                    fuel,
                    pos: self.apos[sup],
                    time: 0,
                    parent: usize::MAX,
                    action: -1,
                    road_cost: 0,
                }];
                let mut buckets = vec![Vec::<usize>::new(); (steps + 1) * n];
                buckets[self.apos[sup]].push(0);
                for t in 0..steps {
                    if self.expired() {
                        return vec![];
                    }
                    for pos in 0..n {
                        let ids = std::mem::take(&mut buckets[t * n + pos]);
                        for id in ids {
                            let old = labels[id];
                            for action in -1..6 {
                                let duration = if action < 0 {
                                    1
                                } else {
                                    self.step_cost[pos] as usize
                                };
                                if t + duration > steps {
                                    continue;
                                }
                                let dest = if action < 0 {
                                    pos
                                } else {
                                    let Some(q) = self.map.neighbors[pos][action as usize] else {
                                        continue;
                                    };
                                    if self.map.cells[q] == 3 {
                                        continue;
                                    }
                                    q
                                };
                                let mut fuel = old.fuel;
                                let mut road_cost = old.road_cost;
                                let mut valid = true;
                                for when in t + 1..=t + duration {
                                    let here = if when == t + duration { dest } else { pos };
                                    if self.map.cells[here] == 1
                                        && road_counts[here] < self.map.problem.jammed_threshold
                                    {
                                        road_cost += if road_counts[here] > 0 { 3 } else { 1 };
                                    }
                                    for &p in &self.patrols {
                                        fuel[p] -= debit[p][when];
                                        if fuel[p] < 0 {
                                            valid = false;
                                            break;
                                        }
                                        if positions[p][when] == here {
                                            fuel[p] = cap;
                                        }
                                    }
                                    if !valid {
                                        break;
                                    }
                                }
                                if !valid {
                                    continue;
                                }
                                let bucket = &mut buckets[(t + duration) * n + dest];
                                if bucket.iter().any(|&i| {
                                    labels[i].road_cost <= road_cost
                                        && self
                                            .patrols
                                            .iter()
                                            .all(|&p| labels[i].fuel[p] >= fuel[p])
                                }) {
                                    continue;
                                }
                                bucket.retain(|&i| {
                                    !(road_cost <= labels[i].road_cost
                                        && self
                                            .patrols
                                            .iter()
                                            .all(|&p| fuel[p] >= labels[i].fuel[p]))
                                });
                                if bucket.len() >= per_cell {
                                    let value = |label: &SupplyLabel| {
                                        self.patrols
                                            .iter()
                                            .map(|&p| f64::from(label.fuel[p]).sqrt())
                                            .sum::<f64>()
                                            - f64::from(label.road_cost) * road_weight
                                    };
                                    let worst = (0..bucket.len())
                                        .min_by(|&a, &b| {
                                            value(&labels[bucket[a]])
                                                .total_cmp(&value(&labels[bucket[b]]))
                                        })
                                        .unwrap();
                                    if value(&SupplyLabel {
                                        fuel,
                                        road_cost,
                                        ..old
                                    }) <= value(&labels[bucket[worst]])
                                    {
                                        continue;
                                    }
                                    bucket.swap_remove(worst);
                                }
                                bucket.push(labels.len());
                                labels.push(SupplyLabel {
                                    fuel,
                                    pos: dest,
                                    time: t + duration,
                                    parent: id,
                                    action,
                                    road_cost,
                                });
                            }
                        }
                    }
                }
                let mut endings: Vec<_> = buckets[steps * n..].iter().flatten().copied().collect();
                let value = |id: usize| {
                    let node = labels[id];
                    let fuel = self
                        .patrols
                        .iter()
                        .map(|&p| f64::from(node.fuel[p]).sqrt())
                        .sum::<f64>();
                    fuel - f64::from(node.road_cost) * road_weight
                        - self
                            .patrols
                            .iter()
                            .map(|&p| {
                                let a = positions[p][steps];
                                let dx = (a % self.map.problem.map.width)
                                    .abs_diff(node.pos % self.map.problem.map.width);
                                let dy = (a / self.map.problem.map.width)
                                    .abs_diff(node.pos / self.map.problem.map.width);
                                (dx + dy) as f64 * f64::from(cap - node.fuel[p]) / f64::from(cap)
                            })
                            .sum::<f64>()
                            * 0.04
                };
                endings.sort_unstable_by(|&a, &b| value(b).total_cmp(&value(a)));
                let mut result = Vec::new();
                let mut distinct = HashSet::new();
                for mut id in endings {
                    let ending = labels[id];
                    let key = (
                        ending.pos,
                        self.patrols
                            .iter()
                            .map(|&p| ending.fuel[p] / 8)
                            .collect::<Vec<_>>(),
                    );
                    if !distinct.insert(key) {
                        continue;
                    }
                    let mut ops = Vec::new();
                    while labels[id].parent != usize::MAX {
                        ops.push(labels[id].action);
                        debug_assert!(labels[labels[id].parent].time < labels[id].time);
                        id = labels[id].parent;
                    }
                    ops.reverse();
                    let mut compressed = Vec::new();
                    for op in ops {
                        if op == -1 && compressed.last().is_some_and(|&x| x < 0) {
                            *compressed.last_mut().unwrap() -= 1;
                        } else {
                            compressed.push(op);
                        }
                    }
                    let mut plan = patrol_plan.clone();
                    plan[sup] = compressed;
                    result.push(plan);
                    if result.len() >= keep {
                        break;
                    }
                }
                result
            }

            fn cover_patrol_plan(&mut self, tours: &Tours, reposition: bool) -> Option<Plan> {
                let mut plan = vec![vec![-self.day_steps]; self.na];
                for p in self.patrols.clone() {
                    let mut ops = Vec::new();
                    let mut pos = self.apos[p];
                    let mut time = 0;
                    if self.map.spot_at[pos].is_some() && self.step_cost[pos] == 1 {
                        ops.push(-1);
                        time += 1;
                    }
                    for &si in &tours[p] {
                        let target = self.map.problem.spots[si].pos;
                        for dir in self.dij(pos).path(pos, target) {
                            time += self.step_cost[pos];
                            ops.push(dir);
                            pos = self.map.neighbors[pos][dir as usize].unwrap();
                        }
                    }
                    if time > self.day_steps {
                        return None;
                    }
                    if reposition && time < self.day_steps {
                        let target = self.map.problem.spots
                            [self.rng.index(self.map.problem.spots.len())]
                        .pos;
                        for dir in self.dij(pos).path(pos, target) {
                            let duration = self.step_cost[pos];
                            if time + duration > self.day_steps {
                                break;
                            }
                            time += duration;
                            ops.push(dir);
                            pos = self.map.neighbors[pos][dir as usize].unwrap();
                        }
                    }
                    if time < self.day_steps {
                        ops.push(time - self.day_steps);
                    }
                    plan[p] = ops;
                }
                Some(plan)
            }

            pub(super) fn cover_beam_select(&mut self) -> Vec<u8> {
                let start = Instant::now();
                let width = config::setting("COVER_BEAM_WIDTH", 24_usize).max(1);
                let samples = config::setting("COVER_BEAM_SAMPLES", 24_usize).max(1);
                let cap = config::setting("COVER_BEAM_LABELS", 24_usize).max(1);
                let mut beam = Vec::new();
                for sup in 0..self.map.problem.agents.len() {
                    let mut kinds = vec![0; self.map.problem.agents.len()];
                    kinds[sup] = 1;
                    let agents = self
                        .map
                        .problem
                        .agents
                        .iter()
                        .enumerate()
                        .map(|(a, &pos)| Agent {
                            kind: kinds[a],
                            pos,
                            fuel: self.map.problem.fuel_limits,
                        })
                        .collect();
                    beam.push(CoverSchedule {
                        kinds,
                        plans: vec![],
                        states: vec![],
                        agents,
                        history: vec![],
                        value: 0.,
                    });
                }
                let mut best = beam[0].clone();
                let mut generated = 0;
                let mut rng = self.rng;
                for day in 0..self.map.problem.day_steps.len() {
                    let mut next = Vec::new();
                    let mut seen = HashSet::new();
                    let mut route_count = 0;
                    let mut supplied_count = 0;
                    let layer_until = (day as u64 + 1) * self.config.assign_ms
                        / self.map.problem.day_steps.len() as u64;
                    'parents: for (parent_index, node) in beam.iter().enumerate() {
                        let now = Self::elapsed(start);
                        let parent_until = now
                            + layer_until.saturating_sub(now) / (beam.len() - parent_index) as u64;
                        let observation =
                            self.horizon_observation(day, node.agents.clone(), &node.history);
                        'variants: for variant in
                            0..config::setting("COVER_BEAM_VARIANTS", 12_usize)
                        {
                            if self.expired() || Self::elapsed(start) >= self.config.assign_ms {
                                break 'parents;
                            }
                            self.path_fuel = if variant < 3 { [0, 4, 12][variant] } else { 0 };
                            self.path_salt = if variant < 3 {
                                0
                            } else {
                                rng.index(255) as u32 + 1
                            };
                            self.distances.fill(None);
                            self.prepare(&observation, self.map.problem.day_steps[day] as i32);
                            self.rng = rng;
                            let tours = self.exact_partition_samples(samples);
                            rng = self.rng;
                            for (reposition, tour) in
                                (0..config::setting("COVER_BEAM_TAILS", 16_usize))
                                    .flat_map(|r| tours.iter().map(move |t| (r, t)))
                            {
                                if self.expired() {
                                    break 'parents;
                                }
                                if Self::elapsed(start) >= parent_until && !next.is_empty() {
                                    break 'variants;
                                }
                                let Some(plan) = self.cover_patrol_plan(tour, reposition != 0)
                                else {
                                    continue;
                                };
                                route_count += 1;
                                for plan in self.cover_supply(&plan, cap, 8) {
                                    supplied_count += 1;
                                    let Some(actual) = self.replay(&plan, &self.apos, &self.afuel)
                                    else {
                                        continue;
                                    };
                                    if actual.claimed.count_ones() != self.map.brands as u32 {
                                        continue;
                                    }
                                    let key = (
                                        node.kinds.clone(),
                                        actual.pos.clone(),
                                        actual.fuel.clone(),
                                        actual.stay.clone(),
                                    );
                                    if !seen.insert(key) {
                                        continue;
                                    }
                                    let mut candidate = node.clone();
                                    candidate.states.push(observation.clone());
                                    candidate.plans.push(plan);
                                    candidate.agents = actual
                                        .pos
                                        .iter()
                                        .enumerate()
                                        .map(|(a, &pos)| Agent {
                                            kind: node.kinds[a],
                                            pos,
                                            fuel: actual.fuel[a] as u32,
                                        })
                                        .collect();
                                    candidate.history.push(actual.stay);
                                    if candidate.history.len() > 2 {
                                        candidate.history.remove(0);
                                    }
                                    candidate.value = self
                                        .patrols
                                        .iter()
                                        .map(|&p| (actual.fuel[p] as f64).sqrt())
                                        .sum();
                                    let busy = self.map.problem.busy_threshold as i32;
                                    let jam = self.map.problem.jammed_threshold as i32;
                                    let traffic_cost = (0..self.map.cells.len())
                                        .filter(|&p| self.map.cells[p] == 1)
                                        .map(|p| {
                                            let count: i32 =
                                                candidate.history.iter().map(|h| h[p]).sum();
                                            if count >= jam {
                                                3
                                            } else if count >= busy {
                                                1
                                            } else {
                                                0
                                            }
                                        })
                                        .sum::<i32>();
                                    candidate.value -= f64::from(traffic_cost)
                                        * config::setting("COVER_BEAM_TRAFFIC", 0.2_f64);
                                    generated += 1;
                                    if candidate.plans.len() > best.plans.len()
                                        || (candidate.plans.len() == best.plans.len()
                                            && candidate.value > best.value)
                                    {
                                        best = candidate.clone();
                                        self.save_cover_schedule(&best);
                                    }
                                    next.push(candidate);
                                }
                            }
                        }
                    }
                    eprintln!(
                        "[cover beam] day={day} routes={route_count} supplied={supplied_count} candidates={} generated={generated} ms={} best_days={}",
                        next.len(),
                        Self::elapsed(start),
                        best.plans.len()
                    );
                    if next.is_empty() {
                        break;
                    }
                    next.sort_unstable_by(|a, b| b.value.total_cmp(&a.value));
                    let mut shapes = HashSet::new();
                    next.retain(|s| {
                        shapes.insert((
                            s.kinds.clone(),
                            s.agents
                                .iter()
                                .map(|a| (a.pos, a.fuel / 10))
                                .collect::<Vec<_>>(),
                            s.history.clone(),
                        ))
                    });
                    // 同じ終点・燃料でも経路が違えば翌日の交通量が違う。
                    if next.len() > width {
                        for i in width / 4..width {
                            let j = i + rng.index(next.len() - i);
                            next.swap(i, j);
                        }
                    }
                    next.truncate(width);
                    beam = next;
                }
                self.kinds = best.kinds.clone();
                self.cover_schedule = Some(best);
                self.kinds.clone()
            }

            fn save_cover_schedule(&self, schedule: &CoverSchedule) {
                if let Ok(path) = std::env::var("PROCON37_COVER_BEAM_OUTPUT") {
                    let value = serde_json::json!({"problem": self.map.problem, "kinds": schedule.kinds, "plans": schedule.plans, "completedDays": schedule.plans.len()});
                    let temporary = format!("{path}.tmp");
                    if std::fs::write(&temporary, value.to_string()).is_ok() {
                        let _ = std::fs::rename(temporary, path);
                    }
                }
            }

            pub(super) fn cover_beam_day(&mut self, observation: &Observation) -> Option<Plan> {
                let schedule = self.cover_schedule.as_ref()?;
                let expected = schedule.states.get(observation.day)?;
                if expected.agents != observation.agents
                    || expected.traffics != observation.traffics
                {
                    return None;
                }
                let plan = schedule.plans[observation.day].clone();
                self.prepare(
                    observation,
                    self.map.problem.day_steps[observation.day] as i32,
                );
                let actual = self.replay(&plan, &self.apos, &self.afuel)?;
                self.brands_ever |= actual.claimed;
                self.road_history.push(actual.stay);
                Some(plan)
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use tester::{engine::Game, generator::generate};

            #[test]
            fn supply_labels_complete_full_first_day_in_the_independent_engine() {
                let problem = generate(0, None).unwrap().problem;
                let kinds = vec![0, 0, 1, 0];
                let original = Game::new(problem.clone(), &kinds);
                let observation = original.observation(0);
                let mut planner = Planner::new(problem.clone(), 7);
                planner.path_fuel = 0;
                planner.prepare(&observation, problem.day_steps[0] as i32);
                let tours = planner.exact_partition_samples(8);
                assert!(!tours.is_empty());
                let mut checked = 0;
                for tour in tours {
                    let Some(patrol) = planner.cover_patrol_plan(&tour, false) else {
                        continue;
                    };
                    for plan in planner.cover_supply(&patrol, 24, 4) {
                        let replay = planner
                            .replay(&plan, &planner.apos, &planner.afuel)
                            .unwrap();
                        let mut game = original.clone();
                        game.play_day(
                            plan.iter()
                                .map(|a| a.iter().map(|&x| i64::from(x)).collect())
                                .collect(),
                        )
                        .unwrap();
                        assert_eq!(game.score.daily_brands, 12);
                        assert_eq!(replay.claimed.count_ones(), 12);
                        assert_eq!(
                            replay.fuel,
                            game.agents
                                .iter()
                                .map(|a| a.fuel as i32)
                                .collect::<Vec<_>>()
                        );
                        assert_eq!(
                            replay.pos,
                            game.agents.iter().map(|a| a.pos).collect::<Vec<_>>()
                        );
                        assert_eq!(
                            planner
                                .horizon_observation(1, game.agents.clone(), &[replay.stay])
                                .traffics,
                            game.traffic()
                        );
                        checked += 1;
                    }
                }
                assert!(checked > 0);
            }
        }
    }

    // 元ファイル: solver/greedy.rs
    mod greedy {
        use super::*;

        #[derive(Default)]
        pub(super) struct DayResult {
            pub newkinds: u32,
            pub claimed: u32,
            pub balls: i32,
        }

        type Target = (usize, i32, i32, i32);

        // Track distinct ranks, including whether either rank has multiple candidates.
        // Unstable sorting does not promise which tied candidate comes first, so ties
        // at the requested cutoff still use the original sorting implementation.
        #[derive(Default)]
        struct TargetLeaders {
            first: Option<Target>,
            second: Option<Target>,
            first_tied: bool,
            second_tied: bool,
            count: usize,
        }

        impl TargetLeaders {
            fn rank(target: Target) -> (i32, std::cmp::Reverse<i32>, i32) {
                (target.1, std::cmp::Reverse(target.2), target.3)
            }

            fn offer(&mut self, target: Target) {
                self.count += 1;
                let rank = Self::rank(target);
                if self.first.is_none_or(|first| rank > Self::rank(first)) {
                    self.second = self.first;
                    self.second_tied = self.first_tied;
                    self.first = Some(target);
                    self.first_tied = false;
                } else if self.first.is_some_and(|first| rank == Self::rank(first)) {
                    self.first_tied = true;
                } else if self.second.is_none_or(|second| rank > Self::rank(second)) {
                    self.second = Some(target);
                    self.second_tied = false;
                } else if self.second.is_some_and(|second| rank == Self::rank(second)) {
                    self.second_tied = true;
                }
            }

            fn unambiguous(&self, randomized: bool) -> bool {
                !self.first_tied && (!randomized || self.count < 2 || !self.second_tied)
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use tester::{
                engine::Game,
                model::{Map, Spot},
            };

            #[test]
            fn unique_target_prefix_agrees_with_full_sort_including_cutoff_ties() {
                let mut rng = Prng::new(37);
                let mut fast = 0;
                let mut fallback = 0;
                for len in 0..=32 {
                    for pattern in 0..4 {
                        for _ in 0..128 {
                            let mut targets: Vec<Target> = (0..len)
                                .map(|i| match pattern {
                                    0 => (i, 1, i as i32, 1),
                                    1 => (i, 1, 1, 1),
                                    _ => (
                                        i,
                                        [0, 1, 3][rng.index(3)],
                                        rng.index(if pattern == 2 { 200 } else { 4 }) as i32,
                                        rng.index(4) as i32 + 1,
                                    ),
                                })
                                .collect();
                            for i in (1..len).rev() {
                                targets.swap(i, rng.index(i + 1));
                            }
                            let mut leaders = TargetLeaders::default();
                            for &target in &targets {
                                leaders.offer(target);
                            }
                            targets.sort_unstable_by(|a, b| {
                                b.1.cmp(&a.1).then(a.2.cmp(&b.2)).then(b.3.cmp(&a.3))
                            });
                            for randomized in [false, true] {
                                if leaders.unambiguous(randomized) {
                                    fast += 1;
                                    assert_eq!(leaders.first, targets.first().copied());
                                    if randomized && len > 1 {
                                        assert_eq!(leaders.second, targets.get(1).copied());
                                    }
                                } else {
                                    fallback += 1;
                                    // A tie must reach one of the positions being selected.
                                    assert!(
                                        targets
                                            .windows(2)
                                            .take(if randomized { 2 } else { 1 })
                                            .any(|pair| pair[0].1 == pair[1].1
                                                && pair[0].2 == pair[1].2
                                                && pair[0].3 == pair[1].3)
                                    );
                                }
                            }
                        }
                    }
                }
                assert!(fast > 1000 && fallback > 1000);
            }

            fn detached_supply_fixture(
                mode: usize,
                duplicate_supply: bool,
            ) -> (Planner, Game, Sim) {
                let mut starts = vec![2, 3, 2];
                let mut kinds = vec![0, 0, 1];
                if duplicate_supply {
                    starts.push(2);
                    kinds.push(1);
                }
                let problem = Problem {
                    starts_at: 0,
                    day_seconds: vec![60; 3],
                    day_steps: vec![8, 8, 4],
                    map: Map {
                        height: 1,
                        width: 7,
                        cells: vec![vec![0; 7]],
                    },
                    spots: vec![
                        Spot {
                            brand: 0,
                            pos: 5,
                            stocks: 1,
                        },
                        Spot {
                            brand: 1,
                            pos: 0,
                            stocks: 1,
                        },
                        Spot {
                            brand: 2,
                            pos: 6,
                            stocks: 1,
                        },
                    ],
                    agents: starts,
                    fuel_limits: 2,
                    players: 1,
                    busy_threshold: 1,
                    jammed_threshold: 3,
                };
                let mut game = Game::new(problem.clone(), &kinds);
                // Burn the second patrol's fuel in a legal first-day round trip.
                let mut prefix = vec![vec![-8]; kinds.len()];
                prefix[1] = vec![2, 5, -4];
                game.play_day(prefix).unwrap();
                assert_eq!(game.agents[1].fuel, 0);
                let mut planner = Planner::new(problem, 0);
                planner.config.supply_mode = mode;
                planner.config.depth = 1;
                planner.prepare(&game.observation(0), 8);
                planner.configure_supply(&vec![Some(0); kinds.len() - 2]);
                let mut tours = vec![vec![]; kinds.len()];
                tours[0] = vec![0];
                tours[1] = vec![1];
                let mut sim = planner.simulate(&tours);
                planner.fill_supply(&mut sim);
                game.play_day(
                    sim.ops
                        .iter()
                        .map(|ops| ops.iter().map(|&op| i64::from(op)).collect())
                        .collect(),
                )
                .unwrap();
                (planner, game, sim)
            }

            #[test]
            fn rollout_does_not_refuel_a_patrol_after_its_supply_has_departed() {
                let (mut planner, mut game, sim) = detached_supply_fixture(2, false);
                assert_eq!(
                    game.agents.iter().map(|a| a.pos).collect::<Vec<_>>(),
                    [5, 0, 0]
                );
                assert_eq!(game.agents[0].fuel, 0);
                assert_eq!(sim.endfuel[0], 0);
                assert_eq!(sim.sup_overnight, [Some(1)]);
                assert!(planner.optimistic[0]);
                let mut old = sim.clone();
                planner.config.rollout_escorts = false;
                planner.rollout(&mut old);
                let mut corrected = sim;
                planner.config.rollout_escorts = true;
                planner.rollout(&mut corrected);
                assert_eq!((old.future, old.future_newkinds), (3, 1));
                assert_eq!((corrected.future, corrected.future_newkinds), (2, 0));
                // The old forecast claims the brand at cell 6. The stranded patrol
                // cannot take even that single move in the independent engine.
                assert!(
                    game.clone()
                        .play_day(vec![vec![2, -2], vec![-4], vec![-4]])
                        .is_err()
                );
                let before = game.score.daily_brands;
                game.play_day(vec![vec![-4]; 3]).unwrap();
                assert_eq!(game.score.daily_brands - before, corrected.future as usize);
            }

            #[test]
            fn forecast_escorts_have_a_real_supply_partner_for_all_supply_modes() {
                for mode in 0..=3 {
                    for duplicate in [false, true] {
                        let (planner, game, sim) = detached_supply_fixture(mode, duplicate);
                        for &patrol in &planner.patrols {
                            if planner.rollout_escorted(&sim, patrol) {
                                assert!(
                                    game.agents
                                        .iter()
                                        .any(|a| a.kind == 1 && a.pos == game.agents[patrol].pos),
                                    "mode={mode} duplicate={duplicate} patrol={patrol}"
                                );
                            }
                        }
                        if duplicate {
                            assert!(planner.rollout_escorted(&sim, 0));
                        }
                        if mode == 2 && !duplicate {
                            assert!(!planner.rollout_escorted(&sim, 0));
                            assert!(planner.rollout_escorted(&sim, 1));
                        }
                    }
                }
            }
        }

        impl Planner {
            #[allow(clippy::too_many_arguments)] // 元実装の訪問集合・通算集合・当日集合を区別する。
            #[inline]
            fn choose_target(
                &mut self,
                pos: usize,
                remaining: i32,
                stock: &[i32],
                visited: u32,
                seen: u32,
                claimed: u32,
                randomized: bool,
            ) -> Option<usize> {
                if self.config.stack_targets {
                    return self.choose_target_storage::<true>(
                        pos, remaining, stock, visited, seen, claimed, randomized,
                    );
                }
                let d = self.dij(pos);
                let mut leaders = TargetLeaders::default();
                let (mut advance, mut advance_tier, mut advance_dist) = (None, -1, INF);
                for (i, spot) in self.map.problem.spots.iter().enumerate() {
                    if stock[i] <= 0
                        || visited & (1 << i) != 0
                        || spot.pos == pos
                        || d.dist[spot.pos] >= INF
                    {
                        continue;
                    }
                    let bit = 1 << spot.brand;
                    let tier = if seen & bit == 0 {
                        3
                    } else if claimed & bit == 0 {
                        1
                    } else {
                        0
                    };
                    let distance = d.dist[spot.pos];
                    if distance <= remaining {
                        leaders.offer((i, tier, distance, stock[i]));
                    }
                    if tier >= 1
                        && (tier > advance_tier
                            || (tier == advance_tier && distance < advance_dist))
                    {
                        advance = Some(i);
                        advance_tier = tier;
                        advance_dist = distance;
                    }
                }
                if !leaders.unambiguous(randomized) {
                    return self.choose_target_storage::<false>(
                        pos, remaining, stock, visited, seen, claimed, randomized,
                    );
                }
                let Some(first) = leaders.first else {
                    return advance;
                };
                // Draw exactly when the full-sort implementation would draw, even if
                // both choices would otherwise appear equally attractive.
                if randomized && leaders.count > 1 && self.rng.index(2) == 1 {
                    Some(leaders.second.unwrap().0)
                } else {
                    Some(first.0)
                }
            }

            #[allow(clippy::too_many_arguments)]
            #[inline(never)] // Keep the unused stack buffer out of the heap variant's frame.
            fn choose_target_storage<const USE_STACK: bool>(
                &mut self,
                pos: usize,
                remaining: i32,
                stock: &[i32],
                visited: u32,
                seen: u32,
                claimed: u32,
                randomized: bool,
            ) -> Option<usize> {
                let d = self.dij(pos);
                // The protocol accepts at most 32 spots. Keep the same input order and
                // slice sort so equal-score targets and random-number consumption agree.
                let use_stack = USE_STACK;
                let mut stack = [(0_usize, 0_i32, 0_i32, 0_i32); 32];
                let mut count = 0;
                let mut heap = Vec::with_capacity(if use_stack { 0 } else { 32 });
                let (mut advance, mut advance_tier, mut advance_dist) = (None, -1, INF);
                for (i, s) in self.map.problem.spots.iter().enumerate() {
                    if stock[i] <= 0
                        || visited & (1 << i) != 0
                        || s.pos == pos
                        || d.dist[s.pos] >= INF
                    {
                        continue;
                    }
                    let bit = 1 << s.brand;
                    let tier = if seen & bit == 0 {
                        3
                    } else if claimed & bit == 0 {
                        1
                    } else {
                        0
                    };
                    let distance = d.dist[s.pos];
                    if distance <= remaining {
                        let candidate = (i, tier, distance, stock[i]);
                        if use_stack {
                            stack[count] = candidate;
                            count += 1;
                        } else {
                            heap.push(candidate);
                        }
                    }
                    if tier >= 1
                        && (tier > advance_tier
                            || (tier == advance_tier && distance < advance_dist))
                    {
                        advance = Some(i);
                        advance_tier = tier;
                        advance_dist = distance;
                    }
                }
                let candidates = if use_stack {
                    &mut stack[..count]
                } else {
                    heap.as_mut_slice()
                };
                candidates
                    .sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)).then(b.3.cmp(&a.3)));
                if candidates.is_empty() {
                    advance
                } else {
                    Some(
                        candidates[if randomized && candidates.len() > 1 {
                            self.rng.index(2)
                        } else {
                            0
                        }]
                        .0,
                    )
                }
            }

            pub(super) fn greedy_init(&mut self, randomized: bool) -> Tours {
                let mut tours = vec![vec![]; self.na];
                let mut stock: Vec<_> = self
                    .map
                    .problem
                    .spots
                    .iter()
                    .map(|s| s.stocks as i32)
                    .collect();
                let mut seen = self.brands_ever;
                let mut claimed = 0;
                for p in self.order.clone() {
                    let (mut pos, mut fuel, mut used) = (self.apos[p], self.afuel[p], 0);
                    let mut escort = self.optimistic[p];
                    let mut rdv_done = false;
                    let mut visited = 0;
                    if self.day_steps > 0 {
                        self.collect(pos, &mut stock, &mut visited, &mut claimed);
                        seen |= claimed;
                    }
                    while used < self.day_steps {
                        if !self.precomputed[pos] {
                            break;
                        }
                        let Some(si) = self.choose_target(
                            pos,
                            self.day_steps - used,
                            &stock,
                            visited,
                            seen,
                            claimed,
                            randomized,
                        ) else {
                            break;
                        };
                        let target = self.map.problem.spots[si].pos;
                        let d = self.dij(pos);
                        let dirs = d.path_view(pos, target);
                        if dirs.is_empty() {
                            break;
                        }
                        let (mut blocked, mut progressed) = (false, false);
                        for &dir in dirs.iter() {
                            let (cost, fc) = (self.step_cost[pos], self.fuel_cost[pos]);
                            if !escort
                                && fuel < fc
                                && !rdv_done
                                && let Some(resume) = self.rendezvous(p, pos, used)
                            {
                                used = resume;
                                fuel = self.map.problem.fuel_limits as i32;
                                escort = true;
                                rdv_done = true;
                            }
                            if used + cost > self.day_steps || (!escort && fuel < fc) {
                                blocked = true;
                                break;
                            }
                            used += cost;
                            if !escort {
                                fuel -= fc;
                            }
                            pos = self.map.neighbors[pos][dir as usize].unwrap();
                            progressed = true;
                            self.collect(pos, &mut stock, &mut visited, &mut claimed);
                            seen |= claimed;
                        }
                        tours[p].push(si);
                        if blocked || !progressed {
                            break;
                        }
                    }
                }
                tours
            }

            pub(super) fn greedy_day(
                &mut self,
                pos: &mut [usize],
                fuel: &mut [i32],
                brands: &mut u32,
                steps: i32,
                randomized: bool,
                day_end: Option<&Sim>,
            ) -> DayResult {
                let mut stock: Vec<_> = self
                    .map
                    .problem
                    .spots
                    .iter()
                    .map(|s| s.stocks as i32)
                    .collect();
                let original_brands = *brands;
                let mut claimed = 0;
                let mut balls = 0;
                for p in self.order.clone() {
                    let (mut cur, mut f, mut used) = (pos[p], fuel[p], 0);
                    let escort =
                        day_end.map_or(self.optimistic[p], |s| self.rollout_escorted(s, p));
                    let mut visited = 0;
                    if steps > 0 && self.collect(cur, &mut stock, &mut visited, &mut claimed) {
                        balls += 1;
                    }
                    *brands |= claimed;
                    while used < steps {
                        let Some(si) = self.choose_target(
                            cur,
                            steps - used,
                            &stock,
                            visited,
                            *brands,
                            claimed,
                            randomized,
                        ) else {
                            break;
                        };
                        let target = self.map.problem.spots[si].pos;
                        let d = self.dij(cur);
                        let dirs = d.path_view(cur, target);
                        if dirs.is_empty() {
                            break;
                        }
                        let (mut blocked, mut progressed) = (false, false);
                        for &dir in dirs.iter() {
                            let (cost, fc) = (self.step_cost[cur], self.fuel_cost[cur]);
                            if used + cost > steps || (!escort && f < fc) {
                                blocked = true;
                                break;
                            }
                            used += cost;
                            if !escort {
                                f -= fc;
                            }
                            cur = self.map.neighbors[cur][dir as usize].unwrap();
                            progressed = true;
                            if self.collect(cur, &mut stock, &mut visited, &mut claimed) {
                                balls += 1;
                            }
                            *brands |= claimed;
                        }
                        if blocked || !progressed {
                            break;
                        }
                    }
                    pos[p] = cur;
                    fuel[p] = f;
                }
                DayResult {
                    newkinds: (*brands & !original_brands).count_ones(),
                    claimed: claimed.count_ones(),
                    balls,
                }
            }

            pub(super) fn rollout(&mut self, s: &mut Sim) {
                s.future = 0;
                s.future_newkinds = 0;
                s.future_balls = 0;
                if self.config.depth == 0 || self.day_index + 1 >= self.map.problem.day_steps.len()
                {
                    return;
                }
                if self.config.reach {
                    let mut reach = 0_u32;
                    let next_steps = self.map.problem.day_steps[self.day_index + 1] as i32;
                    for p in self.patrols.clone() {
                        let start = s.endpos[p];
                        let d = self.dij(start);
                        for spot in &self.map.problem.spots {
                            if d.dist[spot.pos] > next_steps {
                                continue;
                            }
                            let mut cell = spot.pos;
                            let mut cost = 0;
                            while cell != start {
                                cell = d.par[cell];
                                cost += self.fuel_cost[cell];
                            }
                            if cost <= s.endfuel[p] {
                                reach |= 1 << spot.brand;
                            }
                        }
                    }
                    s.future = reach.count_ones();
                    s.future_newkinds = (reach & !(self.brands_ever | s.claimed)).count_ones();
                    return;
                }
                let mut pos = s.endpos.clone();
                let mut fuel = s.endfuel.clone();
                let mut brands = self.brands_ever | s.claimed;
                for offset in 1..=self.config.depth {
                    let day = self.day_index + offset;
                    if day >= self.map.problem.day_steps.len() {
                        break;
                    }
                    let result = self.greedy_day(
                        &mut pos,
                        &mut fuel,
                        &mut brands,
                        self.map.problem.day_steps[day] as i32,
                        false,
                        self.config.rollout_escorts.then_some(&*s),
                    );
                    s.future += result.claimed;
                    s.future_newkinds += result.newkinds;
                    s.future_balls += result.balls;
                }
            }

            fn rollout_escorted(&self, day_end: &Sim, patrol: usize) -> bool {
                day_end.endfuel[patrol] >= INF / 4
                    || day_end.sup_overnight.contains(&Some(patrol))
                    // Multiple supplies can initially follow the same patrol. Only
                    // one is detached in mode 2; another can still follow it all day.
                    || self.supplies.iter().enumerate().any(|(k, &supply)| {
                        self.supply_target[supply] == Some(patrol)
                            && self.apos[supply] == self.apos[patrol]
                            && day_end.sup_detach[k].is_none()
                    })
            }
        }
    }

    // 元ファイル: solver/horizon.rs
    mod horizon {
        //! 全日程の経路を同時に探索する。道路滞在量・燃料は厳密再生で持ち越す。
        use super::*;
        use tester::model::{Agent, Traffic};

        #[derive(Clone)]
        struct Day {
            tours: Tours,
            targets: Vec<Option<usize>>,
            order: Vec<usize>,
            observation: Observation,
            road: Vec<u8>,
            paths: Vec<Option<Arc<Dij>>>,
            path_fuel: i32,
            path_salt: u32,
            plan: Plan,
            stay: Vec<i32>,
            end: Vec<Agent>,
            claimed: u32,
            balls: i32,
            tie: f64,
            meetings: Vec<Vec<(usize, i32)>>,
            holds: Vec<Vec<i32>>,
            tails: Vec<Option<usize>>,
            shortage: i32,
            overtime: i32,
            endfuel: Vec<i32>,
        }

        #[derive(Clone)]
        pub(super) struct Schedule {
            kinds: Vec<u8>,
            days: Vec<Day>,
            value: f64,
            daily: u32,
            unique: u32,
            shortage: i32,
            overtime: i32,
        }

        impl Planner {
            pub(super) fn horizon_observation(
                &self,
                day: usize,
                agents: Vec<Agent>,
                history: &[Vec<i32>],
            ) -> Observation {
                let traffics = self
                    .map
                    .cells
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| **t == 1)
                    .map(|(pos, _)| {
                        let count: i32 = history.iter().rev().take(2).map(|h| h[pos]).sum();
                        Traffic {
                            pos,
                            status: if count >= self.map.problem.jammed_threshold as i32 {
                                2
                            } else if count >= self.map.problem.busy_threshold as i32 {
                                1
                            } else {
                                0
                            },
                        }
                    })
                    .collect();
                Observation {
                    ends_at: 0,
                    day,
                    agents,
                    others: vec![],
                    traffics,
                }
            }

            fn horizon_load(&mut self, day: &Day) {
                let rng = self.rng;
                self.path_fuel = day.path_fuel;
                self.path_salt = day.path_salt;
                self.road = day.road.clone();
                self.distances = day.paths.clone();
                self.prepare(
                    &day.observation,
                    self.map.problem.day_steps[day.observation.day] as i32,
                );
                self.configure_supply(&day.targets);
                self.order = day.order.clone();
                self.rng = rng;
            }

            fn horizon_evaluate(
                &mut self,
                schedule: &mut Schedule,
                first: &Observation,
                from: usize,
                initialize: bool,
            ) -> bool {
                let mut history = self.road_history.clone();
                for day in &schedule.days[..from] {
                    history.push(day.stay.clone());
                }
                let mut agents = if from == 0 {
                    first.agents.clone()
                } else {
                    schedule.days[from - 1].end.clone()
                };
                let count = self.map.problem.day_steps.len() - first.day;
                let saved_rng = self.rng;
                let mut fuel: Vec<i32> = if from == 0 {
                    first.agents.iter().map(|a| a.fuel as i32).collect()
                } else {
                    schedule.days[from - 1].endfuel.clone()
                };
                let mut cluster_groups: Option<Vec<usize>> = None;
                for offset in from..count {
                    let day_index = first.day + offset;
                    let observation = if offset == 0 {
                        first.clone()
                    } else {
                        self.horizon_observation(day_index, agents, &history)
                    };
                    if let Some(old) = schedule.days.get(offset) {
                        self.path_fuel = old.path_fuel;
                        self.path_salt = old.path_salt;
                        self.road = old.road.clone();
                        self.distances = old.paths.clone();
                    } else {
                        self.path_salt = 0;
                        self.path_fuel =
                            if config::setting("CLUSTER_INIT", 0) != 0 && day_index == 0 {
                                0
                            } else {
                                config::setting("PATH_FUEL", 0)
                            };
                        self.distances.fill(None);
                    }
                    self.prepare(&observation, self.map.problem.day_steps[day_index] as i32);
                    self.rng = saved_rng;
                    let (mut tours, targets, order) = if offset >= schedule.days.len() || initialize
                    {
                        let actual_fuel = self.afuel.clone();
                        if config::setting("JOINT", 0) != 0 {
                            self.afuel.fill(1_000_000);
                        }
                        let targets = self.select_supply_targets();
                        self.configure_supply(&targets);
                        let tours = self.greedy_init(true);
                        let saved = self.config.clone();
                        self.config.iterations = config::setting("HORIZON_INIT_ITERS", 1000);
                        self.config.depth = 0;
                        self.config.pool = false;
                        self.config.budget_ms = 200;
                        if config::setting("CLUSTER_INIT", 0) != 0 && self.map.brands <= 16 {
                            if let Some(groups) = &cluster_groups {
                                self.last_tours = self.exact_group_routes(groups);
                            } else if let Some(first_tours) = self.exact_partition() {
                                cluster_groups = Some(
                                    first_tours
                                        .iter()
                                        .map(|t| {
                                            t.iter().fold(0usize, |m, &si| {
                                                m | (1 << self.map.problem.spots[si].brand)
                                            })
                                        })
                                        .collect::<Vec<_>>(),
                                );
                                self.last_tours = first_tours;
                            } else {
                                self.config = saved;
                                self.rng = saved_rng;
                                return false;
                            }
                        } else {
                            self.run_alns(tours, Instant::now());
                        }
                        self.config = saved;
                        self.afuel = actual_fuel;
                        (self.last_tours.clone(), targets, self.order.clone())
                    } else {
                        let old = &schedule.days[offset];
                        self.configure_supply(&old.targets);
                        self.order = old.order.clone();
                        (old.tours.clone(), old.targets.clone(), old.order.clone())
                    };
                    if config::setting("COVER_ALL", 0) != 0 {
                        self.cover_all_tours(&mut tours);
                    }
                    let meetings = if let Some(old) = schedule.days.get(offset) {
                        old.meetings.clone()
                    } else {
                        (0..self.supplies.len())
                            .map(|_| {
                                (0..self.patrols.len() * 2)
                                    .map(|i| {
                                        (
                                            self.patrols[i % self.patrols.len()],
                                            (i as i32 + 1) * self.day_steps
                                                / (self.patrols.len() as i32 * 2 + 1),
                                        )
                                    })
                                    .collect()
                            })
                            .collect()
                    };
                    let holds = schedule
                        .days
                        .get(offset)
                        .map(|d| d.holds.clone())
                        .unwrap_or_else(|| {
                            vec![vec![0; self.map.problem.spots.len() + 1]; self.na]
                        });
                    let tails = schedule
                        .days
                        .get(offset)
                        .map(|d| d.tails.clone())
                        .unwrap_or_else(|| vec![None; self.na]);
                    let joint = config::setting("JOINT", 0) != 0;
                    let actual_steps = self.day_steps;
                    let mut overtime = 0;
                    if joint && config::setting("COVER_ALL", 0) != 0 {
                        let mut longest = actual_steps;
                        for p in self.patrols.clone() {
                            let mut pos = self.apos[p];
                            let mut length = holds[p][0].max(i32::from(
                                self.map.spot_at[pos].is_some() && self.step_cost[pos] == 1,
                            ));
                            for &si in &tours[p] {
                                let next = self.map.problem.spots[si].pos;
                                length += self.dij(pos).dist[next].min(1000) + holds[p][si + 1];
                                pos = next;
                            }
                            overtime += (length - actual_steps).max(0);
                            longest = longest.max(length);
                        }
                        self.day_steps = longest.min(2000);
                    }
                    let plan = if joint {
                        self.joint_plan(&tours, &meetings, &holds, &tails)
                    } else {
                        let mut sim = self.simulate(&tours);
                        self.fill_supply(&mut sim);
                        sim.ops
                    };
                    let Some(actual) = self.replay_with_fuel(&plan, &self.apos, &fuel, joint)
                    else {
                        self.rng = saved_rng;
                        return false;
                    };
                    let mut length = 0;
                    for &p in &self.patrols {
                        let mut pos = self.apos[p];
                        for &si in &tours[p] {
                            let next = self.map.problem.spots[si].pos;
                            length += self.distance(pos, next).min(1000);
                            pos = next;
                        }
                    }
                    let mut missing_distance = 0;
                    for b in 0..self.map.brands {
                        if actual.claimed & (1 << b) != 0 {
                            continue;
                        }
                        let mut near = INF;
                        for p in self.patrols.clone() {
                            let d = self.dij(actual.pos[p]);
                            for spot in &self.map.problem.spots {
                                if spot.brand == b {
                                    near = near.min(d.dist[spot.pos]);
                                }
                            }
                        }
                        missing_distance += near.min(1000);
                    }
                    agents = (0..self.na)
                        .map(|p| Agent {
                            kind: self.kinds[p],
                            pos: actual.pos[p],
                            fuel: actual.fuel[p].max(0) as u32,
                        })
                        .collect();
                    let day = Day {
                        tours,
                        targets,
                        order,
                        observation,
                        road: self.road.clone(),
                        paths: self.distances.clone(),
                        path_fuel: self.path_fuel,
                        path_salt: self.path_salt,
                        plan,
                        meetings,
                        holds,
                        tails,
                        shortage: actual.shortage,
                        overtime,
                        endfuel: actual.fuel.clone(),
                        stay: actual.stay.clone(),
                        end: agents.clone(),
                        claimed: actual.claimed,
                        balls: actual.balls,
                        tie: -(length as f64) * 0.0002 - missing_distance as f64 * 0.002,
                    };
                    fuel = actual.fuel;
                    history.push(actual.stay);
                    if offset < schedule.days.len() {
                        schedule.days[offset] = day;
                    } else {
                        schedule.days.push(day);
                    }
                }
                let ever = schedule
                    .days
                    .iter()
                    .fold(self.brands_ever, |a, d| a | d.claimed);
                schedule.unique = ever.count_ones();
                schedule.daily = schedule.days.iter().map(|d| d.claimed.count_ones()).sum();
                schedule.overtime = schedule.days.iter().map(|d| d.overtime).sum();
                schedule.shortage = schedule.days.iter().map(|d| d.shortage).sum();
                schedule.value = schedule.unique as f64 * 100.0
                    + schedule.daily as f64
                    + schedule.days.iter().map(|d| d.tie).sum::<f64>()
                    - schedule.shortage as f64 * config::setting("FUEL_PENALTY", 0.25_f64)
                    - schedule.overtime as f64 * config::setting("TIME_PENALTY", 0.15_f64);
                self.rng = saved_rng;
                true
            }

            fn horizon_better(a: &Schedule, b: &Schedule) -> bool {
                if config::setting("COVER_ALL", 0) != 0 {
                    return a.value > b.value;
                }
                (std::cmp::Reverse(a.shortage), a.unique, a.daily)
                    > (std::cmp::Reverse(b.shortage), b.unique, b.daily)
                    || ((a.shortage, a.unique, a.daily) == (b.shortage, b.unique, b.daily)
                        && a.value > b.value)
            }

            fn horizon_search(
                &mut self,
                mut best: Schedule,
                observation: &Observation,
                budget: u64,
            ) -> Schedule {
                let start = Instant::now();
                let mut current = best.clone();
                let mut iterations = 0;
                let session_ms = config::setting("HORIZON_SESSION_MS", 4000_u64).max(100);
                let mut session = 0;
                while Self::elapsed(start) < budget && !self.expired() {
                    if config::setting("COVER_ALL", 0) != 0
                        && best.shortage == 0
                        && best.overtime == 0
                        && best.daily == (self.map.brands * best.days.len()) as u32
                    {
                        break;
                    }
                    iterations += 1;
                    let next_session = Self::elapsed(start) / session_ms;
                    if next_session != session {
                        session = next_session;
                        current = best.clone();
                    }
                    let mut candidate = current.clone();
                    let day = self.rng.index(candidate.days.len());
                    self.horizon_load(&candidate.days[day]);
                    let op = self.rng.index(100);
                    if op < 10 && config::setting("JOINT", 0) != 0 {
                        let p = self.patrols[self.rng.index(self.patrols.len())];
                        let nodes = &candidate.days[day].tours[p];
                        let i = self.rng.index(nodes.len() + 1);
                        let index = if i == 0 { 0 } else { nodes[i - 1] + 1 };
                        let hold = &mut candidate.days[day].holds[p][index];
                        *hold = if self.rng.next() < 0.3 {
                            0
                        } else {
                            (*hold + self.rng.index(13) as i32 - 6).clamp(0, self.day_steps / 2)
                        };
                    } else if op < 40
                        && config::setting("JOINT", 0) != 0
                        && !self.supplies.is_empty()
                    {
                        let k = self.rng.index(self.supplies.len());
                        let list = &mut candidate.days[day].meetings[k];
                        let i = self.rng.index(list.len().max(1));
                        let p = self.patrols[self.rng.index(self.patrols.len())];
                        let t = self.rng.index(self.day_steps as usize + 1) as i32;
                        match self.rng.index(5) {
                            0 if !list.is_empty() => {
                                list.remove(i);
                            }
                            1 if !list.is_empty() => {
                                list[i].0 = p;
                            }
                            2 if !list.is_empty() => {
                                list[i].1 = (list[i].1 + self.rng.index(17) as i32 - 8)
                                    .clamp(0, self.day_steps);
                            }
                            3 if !list.is_empty() => {
                                list[i].1 = t;
                            }
                            _ => {
                                if list.len() < 16 {
                                    list.push((p, t));
                                }
                            }
                        }
                        list.sort_by_key(|m| m.1);
                    } else if op < 65 {
                        for _ in 0..(1 + self.rng.index(3)) {
                            self.mutate(&mut candidate.days[day].tours, false);
                        }
                    } else if op < 67 && config::setting("PATH_VARIANTS", 0) != 0 {
                        candidate.days[day].path_salt = self.rng.index(256) as u32;
                        candidate.days[day].paths.fill(None);
                    } else if op < 69 && config::setting("DAY_PATHS", 0) != 0 {
                        let weights = [0, 1, 2, 4, 6, 8, 12, 20, 40];
                        candidate.days[day].path_fuel = weights[self.rng.index(weights.len())];
                        candidate.days[day].paths.fill(None);
                    } else if op < 73 && config::setting("JOINT", 0) != 0 {
                        let p = self.rng.index(self.na);
                        candidate.days[day].tails[p] = if self.rng.next() < 0.25 {
                            None
                        } else {
                            Some(self.rng.index(self.map.problem.spots.len()))
                        };
                    } else if op < 80 {
                        let d = self.rng.index(5);
                        let r = self.rng.index(3);
                        let q = self.destroy_degree();
                        self.destroy_repair(&mut candidate.days[day].tours, d, r, q, false);
                    } else if op < 90 && self.patrols.len() > 1 {
                        let i = self.rng.index(self.patrols.len());
                        let j = self.rng.index(self.patrols.len());
                        candidate.days[day].order.swap(i, j);
                        if config::setting("JOINT", 0) != 0 {
                            let p = self.patrols[i];
                            let q = self.patrols[j];
                            let last = if self.rng.next() < 0.5 {
                                candidate.days.len()
                            } else {
                                day + 1
                            };
                            for later in &mut candidate.days[day..last] {
                                later.tours.swap(p, q);
                                later.holds.swap(p, q);
                                later.tails.swap(p, q);
                                for list in &mut later.meetings {
                                    for m in list {
                                        if m.0 == p {
                                            m.0 = q;
                                        } else if m.0 == q {
                                            m.0 = p;
                                        }
                                    }
                                }
                            }
                        }
                        if !self.supplies.is_empty() {
                            let k = self.rng.index(self.supplies.len());
                            candidate.days[day].targets[k] = Some(self.patrols[i]);
                        }
                    } else if op < 95 && candidate.days.len() > 1 {
                        let source = self.rng.index(candidate.days.len());
                        let p = self.patrols[self.rng.index(self.patrols.len())];
                        candidate.days[day].tours[p] = candidate.days[source].tours[p]
                            .iter()
                            .rev()
                            .copied()
                            .collect();
                    } else {
                        candidate.days[day].tours = self.greedy_init(true);
                    }
                    if config::setting("COVER_ALL", 0) != 0 {
                        self.cover_all_tours(&mut candidate.days[day].tours);
                    }
                    if config::setting("ROUTE_DP", 0) != 0 && self.rng.next() < 0.3 {
                        let p = self.patrols[self.rng.index(self.patrols.len())];
                        candidate.days[day].tours[p] =
                            self.reorder_group(p, &candidate.days[day].tours[p]);
                    }
                    if !self.horizon_evaluate(&mut candidate, observation, day, false) {
                        continue;
                    }
                    let fraction = (Self::elapsed(start) % session_ms) as f64 / session_ms as f64;
                    let t0: f64 = config::setting("HORIZON_T0", 1.5);
                    let t1: f64 = config::setting("HORIZON_T1", 0.03);
                    let temp = t0 * (t1 / t0).powf(fraction);
                    if candidate.value >= current.value
                        || self.rng.next() < ((candidate.value - current.value) / temp).exp()
                    {
                        current = candidate.clone();
                    }
                    if Self::horizon_better(&candidate, &best) {
                        best = candidate;
                        self.write_horizon_checkpoint(&best, observation.day);
                        if self.config.debug > 0 {
                            eprintln!(
                                "[horizon] it={iterations} daily={} days={:?} value={:.3} shortage={} overtime={}",
                                best.daily,
                                best.days
                                    .iter()
                                    .map(|d| d.claimed.count_ones())
                                    .collect::<Vec<_>>(),
                                best.value,
                                best.shortage,
                                best.overtime
                            );
                        }
                    }
                }
                eprintln!(
                    "[horizon done] it={iterations} ms={} daily={} days={:?} balls={} shortage={} overtime={}",
                    Self::elapsed(start),
                    best.daily,
                    best.days
                        .iter()
                        .map(|d| d.claimed.count_ones())
                        .collect::<Vec<_>>(),
                    best.days.iter().map(|d| d.balls).sum::<i32>(),
                    best.shortage,
                    best.overtime
                );
                self.write_horizon_checkpoint(&best, observation.day);
                best
            }

            fn write_horizon_checkpoint(&self, best: &Schedule, day_index: usize) {
                if let Ok(directory) = std::env::var("PROCON37_DUMP_DIR") {
                    let path = std::path::Path::new(&directory);
                    let _ = std::fs::create_dir_all(path);
                    let value = serde_json::json!({"problem": self.map.problem, "kinds": best.kinds,
                        "daily": best.daily, "shortage": best.shortage, "overtime": best.overtime,
                        "days": best.days.iter().map(|d| serde_json::json!({
                            "day": d.observation.day, "pathFuel": d.path_fuel, "pathSalt": d.path_salt, "tours": d.tours, "meetings": d.meetings, "holds": d.holds, "tails": d.tails,
                            "plan": d.plan, "claimed": d.claimed, "fuel": d.endfuel,
                            "targets": d.targets, "order": d.order, "observation": d.observation
                        })).collect::<Vec<_>>()});
                    let name = format!(
                        "{}-{}-{}.json",
                        self.map.problem.map.width,
                        best.kinds
                            .iter()
                            .map(|k| char::from(b'0' + *k))
                            .collect::<String>(),
                        day_index
                    );
                    let temporary = path.join(format!("{name}.tmp"));
                    if std::fs::write(&temporary, value.to_string()).is_ok() {
                        let _ = std::fs::rename(temporary, path.join(name));
                    }
                }
            }

            pub(super) fn horizon_select(&mut self) -> Vec<u8> {
                let start = Instant::now();
                let n = self.map.problem.agents.len();
                if let Some(mut schedule) = self.read_horizon_checkpoint() {
                    let observation = schedule.days[0].observation.clone();
                    self.kinds = schedule.kinds.clone();
                    if self.horizon_evaluate(&mut schedule, &observation, 0, false) {
                        schedule =
                            self.horizon_search(schedule, &observation, self.config.assign_ms);
                        self.kinds = schedule.kinds.clone();
                        self.horizon = Some(schedule);
                        return self.kinds.clone();
                    }
                }
                let forced = config::setting("NUM_SUPPLY", -1_i32);
                let mut candidates = vec![];
                for mask in 0_u32..1 << n {
                    let count = mask.count_ones() as i32;
                    if count > if n >= 7 { 2 } else { 1 } || (forced >= 0 && forced != count) {
                        continue;
                    }
                    candidates.push((0..n).map(|p| ((mask >> p) & 1) as u8).collect::<Vec<_>>());
                }
                let mut best: Option<Schedule> = None;
                let per_candidate = (self.config.assign_ms / (candidates.len() as u64 + 2)).max(20);
                for kinds in candidates {
                    if self.expired() || Self::elapsed(start) >= self.config.assign_ms {
                        break;
                    }
                    self.kinds = kinds.clone();
                    let agents = self
                        .map
                        .problem
                        .agents
                        .iter()
                        .enumerate()
                        .map(|(p, &pos)| Agent {
                            kind: kinds[p],
                            pos,
                            fuel: self.map.problem.fuel_limits,
                        })
                        .collect();
                    let observation = self.horizon_observation(0, agents, &[]);
                    let mut schedule = Schedule {
                        kinds: kinds.clone(),
                        days: vec![],
                        value: 0.,
                        daily: 0,
                        unique: 0,
                        shortage: 0,
                        overtime: 0,
                    };
                    if !self.horizon_evaluate(&mut schedule, &observation, 0, true) {
                        continue;
                    }
                    schedule = self.horizon_search(schedule, &observation, per_candidate);
                    eprintln!("[horizon assign] kinds={kinds:?} daily={}", schedule.daily);
                    if best
                        .as_ref()
                        .is_none_or(|b| Self::horizon_better(&schedule, b))
                    {
                        best = Some(schedule);
                    }
                }
                if let Some(mut schedule) = best {
                    let observation = schedule.days[0].observation.clone();
                    self.kinds = schedule.kinds.clone();
                    schedule = self.horizon_search(
                        schedule,
                        &observation,
                        self.config.assign_ms.saturating_sub(Self::elapsed(start)),
                    );
                    self.kinds = schedule.kinds.clone();
                    self.horizon = Some(schedule);
                } else {
                    self.kinds = vec![0; n];
                }
                self.kinds.clone()
            }

            pub(super) fn horizon_day(&mut self, observation: &Observation, steps: i32) -> Plan {
                let start = Instant::now();
                let kinds: Vec<_> = observation.agents.iter().map(|a| a.kind).collect();
                let mut schedule = self.horizon.take().unwrap_or(Schedule {
                    kinds,
                    days: vec![],
                    value: 0.,
                    daily: 0,
                    unique: 0,
                    shortage: 0,
                    overtime: 0,
                });
                schedule
                    .days
                    .retain(|d| d.observation.day >= observation.day);
                let initialize = schedule.days.is_empty();
                if !self.horizon_evaluate(&mut schedule, observation, 0, initialize) {
                    return self.plan_day_local(observation, steps);
                }
                schedule = self.horizon_search(
                    schedule,
                    observation,
                    self.config.budget_ms.saturating_sub(Self::elapsed(start)),
                );
                let day = &schedule.days[0];
                self.horizon_load(day);
                if self.replay(&day.plan, &self.apos, &self.afuel).is_none() {
                    eprintln!("[horizon fallback] shortage={}", schedule.shortage);
                    let plan = self.plan_day_local(observation, steps);
                    if let Some(actual) = self.replay(&plan, &self.apos, &self.afuel) {
                        self.road_history.push(actual.stay);
                    }
                    self.horizon = None;
                    return plan;
                }
                self.brands_ever |= day.claimed;
                self.road_history.push(day.stay.clone());
                let plan = day.plan.clone();
                self.horizon = Some(schedule);
                plan
            }
        }

        impl Planner {
            /// 燃料による切り捨てをせず巡回経路を作り、補給の合流時刻を独立に指定する。
            fn joint_plan(
                &mut self,
                tours: &Tours,
                meetings: &[Vec<(usize, i32)>],
                holds: &[Vec<i32>],
                tails: &[Option<usize>],
            ) -> Plan {
                let steps = self.day_steps;
                let mut plans = vec![vec![]; self.na];
                let mut positions = vec![vec![0; steps as usize + 1]; self.na];
                for p in self.patrols.clone() {
                    let mut pos = self.apos[p];
                    let mut time = 0;
                    positions[p].fill(pos);
                    let start_hold = holds[p][0].min(steps);
                    if start_hold > 0 {
                        plans[p].push(-start_hold);
                        time = start_hold;
                    }
                    // Starting on a road spot must include a collection step before leaving.
                    if time == 0 && self.map.spot_at[pos].is_some() && self.step_cost[pos] == 1 {
                        plans[p].push(-1);
                        time = 1;
                    }
                    for &si in &tours[p] {
                        let target = self.map.problem.spots[si].pos;
                        let path = self.dij(pos).path(pos, target);
                        for dir in path {
                            let cost = self.step_cost[pos];
                            if time + cost > steps {
                                break;
                            }
                            positions[p][time as usize + 1..(time + cost) as usize].fill(pos);
                            time += cost;
                            pos = self.map.neighbors[pos][dir as usize].unwrap();
                            positions[p][time as usize] = pos;
                            plans[p].push(dir);
                        }
                        if pos != target {
                            break;
                        }
                        let wait = holds[p][si + 1].min(steps - time);
                        if wait > 0 {
                            positions[p][time as usize + 1..=(time + wait) as usize].fill(pos);
                            plans[p].push(-wait);
                            time += wait;
                        }
                    }
                    if let Some(si) = tails[p] {
                        let target = self.map.problem.spots[si].pos;
                        for dir in self.dij(pos).path(pos, target) {
                            let cost = self.step_cost[pos];
                            if time + cost > steps {
                                break;
                            }
                            positions[p][time as usize + 1..(time + cost) as usize].fill(pos);
                            time += cost;
                            pos = self.map.neighbors[pos][dir as usize].unwrap();
                            positions[p][time as usize] = pos;
                            plans[p].push(dir);
                        }
                    }
                    positions[p][time as usize + 1..].fill(pos);
                    if time < steps {
                        plans[p].push(time - steps);
                    }
                }
                let fast_supply = config::setting("FAST_SUPPLY", 0) != 0;
                let mut supply_paths = std::collections::HashMap::new();
                for (k, sup) in self.supplies.clone().into_iter().enumerate() {
                    let mut pos = self.apos[sup];
                    let mut time = 0;
                    for &(p, desired) in &meetings[k] {
                        let d = if fast_supply {
                            supply_paths
                                .entry(pos)
                                .or_insert_with(|| {
                                    Arc::new(Dij::with_fuel(pos, &self.map, &self.road, 0))
                                })
                                .clone()
                        } else {
                            self.dij(pos)
                        };
                        let mut intercept = None;
                        for t in desired.max(time + 1)..=steps {
                            let target = positions[p][t as usize];
                            if d.dist[target] <= t - time {
                                intercept = Some((t, target));
                                break;
                            }
                        }
                        let Some((t, target)) = intercept else {
                            continue;
                        };
                        plans[sup].extend(d.path(pos, target));
                        time += d.dist[target];
                        pos = target;
                        if t > time {
                            plans[sup].push(time - t);
                            time = t;
                        }
                    }
                    if let Some(si) = tails[sup] {
                        let target = self.map.problem.spots[si].pos;
                        let d = if fast_supply {
                            supply_paths
                                .entry(pos)
                                .or_insert_with(|| {
                                    Arc::new(Dij::with_fuel(pos, &self.map, &self.road, 0))
                                })
                                .clone()
                        } else {
                            self.dij(pos)
                        };
                        for dir in d.path(pos, target) {
                            let cost = self.step_cost[pos];
                            if time + cost > steps {
                                break;
                            }
                            time += cost;
                            pos = self.map.neighbors[pos][dir as usize].unwrap();
                            plans[sup].push(dir);
                        }
                    }
                    if time < steps {
                        plans[sup].push(time - steps);
                    }
                }
                plans
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use tester::{engine::Game, generator::generate, model::MatchFormat};

            #[test]
            fn forecast_traffic_uses_full_counts_from_both_days() {
                let problem = generate(0, Some(MatchFormat::First)).unwrap().problem;
                let planner = Planner::new(problem.clone(), 0);
                let road = planner.map.cells.iter().position(|&t| t == 1).unwrap();
                let mut earlier = vec![0; planner.map.cells.len()];
                let mut last = earlier.clone();
                earlier[road] = problem.jammed_threshold as i32 - 1;
                last[road] = 1;
                let observation = planner.horizon_observation(2, vec![], &[earlier, last]);
                assert_eq!(
                    observation
                        .traffics
                        .iter()
                        .find(|t| t.pos == road)
                        .unwrap()
                        .status,
                    2
                );
            }

            #[test]
            fn joint_routes_and_forecast_match_the_independent_engine() {
                let problem = generate(0, Some(MatchFormat::First)).unwrap().problem;
                let kinds = vec![0, 1, 0, 0];
                let mut game = Game::new(problem.clone(), &kinds);
                let mut planner = Planner::new(problem, 0);
                let mut history = vec![];
                for _ in 0..3 {
                    let observation = game.observation(0);
                    planner.prepare(&observation, 12);
                    let tours = vec![vec![0, 2], vec![], vec![10], vec![15]];
                    let plan = planner.joint_plan(
                        &tours,
                        &[vec![(0, 4), (2, 10)]],
                        &vec![vec![0; 17]; 4],
                        &[None; 4],
                    );
                    let actual = planner
                        .replay_with_fuel(&plan, &planner.apos, &planner.afuel, true)
                        .unwrap();
                    if actual.shortage != 0 {
                        break;
                    }
                    game.problem.day_steps[game.day] = 12;
                    game.play_day(
                        plan.iter()
                            .map(|a| a.iter().map(|&x| i64::from(x)).collect())
                            .collect(),
                    )
                    .unwrap();
                    assert_eq!(
                        actual.pos,
                        game.agents.iter().map(|a| a.pos).collect::<Vec<_>>()
                    );
                    assert_eq!(
                        actual.fuel,
                        game.agents
                            .iter()
                            .map(|a| a.fuel as i32)
                            .collect::<Vec<_>>()
                    );
                    history.push(actual.stay);
                    let forecast =
                        planner.horizon_observation(game.day, game.agents.clone(), &history);
                    assert_eq!(forecast.traffics, game.traffic());
                }
            }
        }

        impl Planner {
            fn cover_all_tours(&mut self, tours: &mut Tours) {
                let mut present = 0_u32;
                for p in self.patrols.clone() {
                    tours[p].retain(|&si| {
                        let bit = 1 << self.map.problem.spots[si].brand;
                        let keep = present & bit == 0;
                        present |= bit;
                        keep
                    });
                }
                for brand in 0..self.map.brands {
                    if present & (1 << brand) != 0 {
                        continue;
                    }
                    let mut options = vec![];
                    for (si, spot) in self.map.problem.spots.iter().enumerate() {
                        if spot.brand != brand || spot.stocks == 0 {
                            continue;
                        }
                        for &p in &self.patrols {
                            for i in 0..=tours[p].len() {
                                let prev = if i == 0 {
                                    self.apos[p]
                                } else {
                                    self.map.problem.spots[tours[p][i - 1]].pos
                                };
                                let extra = self.distance(prev, spot.pos)
                                    + tours[p].get(i).map_or(0, |&next| {
                                        let next = self.map.problem.spots[next].pos;
                                        self.distance(spot.pos, next) - self.distance(prev, next)
                                    });
                                options.push((extra, p, i, si));
                            }
                        }
                    }
                    options.sort_unstable();
                    if !options.is_empty() {
                        let pick = if self.rng.next() < 0.7 {
                            0
                        } else {
                            self.rng.index(options.len().min(4))
                        };
                        let (_, p, i, si) = options[pick];
                        tours[p].insert(i, si);
                    }
                }
            }
        }

        impl Planner {
            /// 実験を継続するための任意指定チェックポイント。問題全体の一致を確認する。
            fn read_horizon_checkpoint(&self) -> Option<Schedule> {
                let path = std::env::var("PROCON37_HORIZON_LOAD").ok()?;
                let value: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
                let problem: Problem =
                    serde_json::from_value(value.get("problem")?.clone()).ok()?;
                if problem != self.map.problem {
                    return None;
                }
                let kinds: Vec<u8> = serde_json::from_value(value.get("kinds")?.clone()).ok()?;
                let n = self.map.problem.agents.len();
                let ns = self.map.problem.spots.len();
                if kinds.len() != n || kinds.iter().any(|&k| k > 1) || !kinds.contains(&0) {
                    return None;
                }
                let mut days = vec![];
                for (i, d) in value.get("days")?.as_array()?.iter().enumerate() {
                    let observation: Observation =
                        serde_json::from_value(d.get("observation")?.clone()).ok()?;
                    if observation.day != i {
                        return None;
                    }
                    let tours: Tours = serde_json::from_value(d.get("tours")?.clone()).ok()?;
                    let meetings: Vec<Vec<(usize, i32)>> =
                        serde_json::from_value(d.get("meetings")?.clone()).ok()?;
                    let holds: Vec<Vec<i32>> = d
                        .get("holds")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_else(|| vec![vec![0; ns + 1]; n]);
                    let tails: Vec<Option<usize>> = d
                        .get("tails")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_else(|| vec![None; n]);
                    if tails.len() != n || tails.iter().flatten().any(|&si| si >= ns) {
                        return None;
                    }
                    let targets: Vec<Option<usize>> =
                        serde_json::from_value(d.get("targets")?.clone()).ok()?;
                    let order: Vec<usize> = serde_json::from_value(d.get("order")?.clone()).ok()?;
                    if tours.len() != n
                        || tours.iter().flatten().any(|&si| si >= ns)
                        || meetings.len() != kinds.iter().filter(|&&k| k == 1).count()
                        || meetings
                            .iter()
                            .flatten()
                            .any(|&(p, t)| p >= n || kinds[p] != 0 || t < 0 || t > 2000)
                        || holds.len() != n
                        || holds.iter().any(|a| {
                            a.len() != ns + 1 || a.iter().any(|&x| !(0..=2000).contains(&x))
                        })
                        || targets.len() != meetings.len()
                        || targets.iter().flatten().any(|&p| p >= n || kinds[p] != 0)
                        || order.iter().any(|&p| p >= n || kinds[p] != 0)
                    {
                        return None;
                    }
                    days.push(Day {
                        tours,
                        targets,
                        order,
                        observation,
                        meetings,
                        holds,
                        tails,
                        road: vec![0; self.map.cells.len()],
                        paths: vec![None; self.map.cells.len()],
                        path_salt: d.get("pathSalt").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        path_fuel: d
                            .get("pathFuel")
                            .and_then(|v| v.as_i64())
                            .map(|x| x as i32)
                            .unwrap_or_else(|| config::setting("PATH_FUEL", 0)),
                        plan: vec![],
                        stay: vec![],
                        end: vec![],
                        claimed: 0,
                        balls: 0,
                        tie: 0.,
                        shortage: 0,
                        overtime: 0,
                        endfuel: vec![],
                    });
                }
                if days.len() != self.map.problem.day_steps.len() {
                    return None;
                }
                Some(Schedule {
                    kinds,
                    days,
                    value: 0.,
                    daily: 0,
                    unique: 0,
                    shortage: 0,
                    overtime: 0,
                })
            }
        }
    }

    // 元ファイル: solver/neighborhood.rs
    mod neighborhood {
        use super::*;

        impl Planner {
            fn pick_nonempty(&mut self, tours: &Tours) -> Option<usize> {
                for _ in 0..8 {
                    let p = self.patrols[self.rng.index(self.patrols.len())];
                    if !tours[p].is_empty() {
                        return Some(p);
                    }
                }
                None
            }
            pub(super) fn mutate(&mut self, tours: &mut Tours, no_remove: bool) {
                if self.patrols.is_empty() {
                    return;
                }
                let mut used = vec![false; self.map.problem.spots.len()];
                for &p in &self.patrols {
                    for &si in &tours[p] {
                        used[si] = true;
                    }
                }
                let unused: Vec<_> = (0..used.len())
                    .filter(|&si| !used[si] && self.map.problem.spots[si].stocks > 0)
                    .collect();
                let mut op = self.rng.index(if no_remove {
                    if self.config.oropt { 5 } else { 4 }
                } else if self.config.oropt {
                    6
                } else {
                    5
                });
                if no_remove && op == 4 {
                    op = 5;
                }
                match op {
                    0 => {
                        let Some(p) = self.pick_nonempty(tours) else {
                            return;
                        };
                        let i = self.rng.index(tours[p].len());
                        let x = tours[p].remove(i);
                        let q = self.patrols[self.rng.index(self.patrols.len())];
                        let j = self.rng.index(tours[q].len() + 1);
                        tours[q].insert(j, x);
                    }
                    1 => {
                        let p = self.pick_nonempty(tours);
                        let q = self.pick_nonempty(tours);
                        let (Some(p), Some(q)) = (p, q) else {
                            return;
                        };
                        let i = self.rng.index(tours[p].len());
                        let j = self.rng.index(tours[q].len());
                        let a = tours[p][i];
                        tours[p][i] = tours[q][j];
                        tours[q][j] = a;
                    }
                    2 => {
                        let Some(p) = self.pick_nonempty(tours) else {
                            return;
                        };
                        if tours[p].len() < 2 {
                            return;
                        }
                        let i = self.rng.index(tours[p].len());
                        let j = self.rng.index(tours[p].len());
                        tours[p][i.min(j)..=i.max(j)].reverse();
                    }
                    3 => {
                        if unused.is_empty() {
                            return;
                        }
                        let x = unused[self.rng.index(unused.len())];
                        let q = self.patrols[self.rng.index(self.patrols.len())];
                        let j = self.rng.index(tours[q].len() + 1);
                        tours[q].insert(j, x);
                    }
                    4 => {
                        let Some(p) = self.pick_nonempty(tours) else {
                            return;
                        };
                        let i = self.rng.index(tours[p].len());
                        tours[p].remove(i);
                    }
                    _ => {
                        let Some(p) = self.pick_nonempty(tours) else {
                            return;
                        };
                        if tours[p].len() < 2 {
                            return;
                        }
                        let n = tours[p].len();
                        let len = n.min(2 + self.rng.index(2));
                        let begin = self.rng.index(n - len + 1);
                        let segment: Vec<_> = tours[p].drain(begin..begin + len).collect();
                        let q = self.patrols[self.rng.index(self.patrols.len())];
                        let j = self.rng.index(tours[q].len() + 1);
                        tours[q].splice(j..j, segment);
                    }
                }
            }
            pub(super) fn destroy_degree(&mut self) -> usize {
                1 + self
                    .rng
                    .index((self.map.problem.spots.len() / 3).clamp(2, 6))
            }
            pub(super) fn unified_degree(&mut self) -> usize {
                let u = self.rng.next();
                1 + (u * u * (self.map.problem.spots.len() / 2).clamp(3, 15) as f64) as usize
            }
            fn present(&self, tours: &Tours) -> Vec<usize> {
                let mut seen = 0_u32;
                let mut result = vec![];
                for &p in &self.patrols {
                    for &si in &tours[p] {
                        if seen & (1 << si) == 0 {
                            seen |= 1 << si;
                            result.push(si);
                        }
                    }
                }
                result
            }
            fn destroy_mask(&mut self, tours: &Tours, op: usize, q: usize) -> Vec<bool> {
                let mut mask = vec![false; self.map.problem.spots.len()];
                match op {
                    0 => {
                        let mut present = self.present(tours);
                        for _ in 0..q {
                            if present.is_empty() {
                                break;
                            }
                            let i = self.rng.index(present.len());
                            mask[present.remove(i)] = true;
                        }
                    }
                    1 => {
                        let mut counts = vec![0; self.map.brands];
                        for &p in &self.patrols {
                            for &si in &tours[p] {
                                counts[self.map.problem.spots[si].brand] += 1;
                            }
                        }
                        let mut entries = vec![];
                        for &p in &self.patrols {
                            for (j, &si) in tours[p].iter().enumerate() {
                                let prev = if j == 0 {
                                    self.apos[p]
                                } else {
                                    self.map.problem.spots[tours[p][j - 1]].pos
                                };
                                let this = self.map.problem.spots[si].pos;
                                let next =
                                    tours[p].get(j + 1).map(|&x| self.map.problem.spots[x].pos);
                                if !self.precomputed[prev] {
                                    continue;
                                }
                                let dprev = self.distance(prev, this);
                                let dnext = next.map_or(0, |n| self.distance(this, n));
                                let skip = next.map_or(0, |n| self.distance(prev, n));
                                let detour = if dprev >= INF {
                                    INF / 4
                                } else {
                                    dprev + dnext - skip
                                };
                                entries.push((
                                    si,
                                    detour as f64
                                        * if counts[self.map.problem.spots[si].brand] > 1 {
                                            3.0
                                        } else {
                                            1.0
                                        },
                                ));
                            }
                        }
                        entries.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
                        for _ in 0..q {
                            if entries.is_empty() {
                                break;
                            }
                            let u = self.rng.next();
                            let i = ((u.powi(2) * entries.len() as f64) as usize)
                                .min(entries.len() - 1);
                            mask[entries.remove(i).0] = true;
                        }
                    }
                    2 => {
                        let present = self.present(tours);
                        if present.is_empty() {
                            return mask;
                        }
                        let seed = present[self.rng.index(present.len())];
                        mask[seed] = true;
                        let mut removed = vec![seed];
                        for _ in 1..q.min(present.len()) {
                            let mut best = None;
                            let mut distance = INF;
                            for &si in &present {
                                if mask[si] {
                                    continue;
                                }
                                let nearest = removed
                                    .iter()
                                    .map(|&r| {
                                        self.distance(
                                            self.map.problem.spots[si].pos,
                                            self.map.problem.spots[r].pos,
                                        )
                                    })
                                    .min()
                                    .unwrap_or(INF);
                                if nearest < distance {
                                    best = Some(si);
                                    distance = nearest;
                                }
                            }
                            let Some(si) = best else {
                                break;
                            };
                            mask[si] = true;
                            removed.push(si);
                        }
                    }
                    3 => {
                        let agents: Vec<_> = self
                            .patrols
                            .iter()
                            .copied()
                            .filter(|&p| !tours[p].is_empty())
                            .collect();
                        if !agents.is_empty() {
                            let p = agents[self.rng.index(agents.len())];
                            for &si in &tours[p] {
                                mask[si] = true;
                            }
                        }
                    }
                    _ => {
                        let agents: Vec<_> = self
                            .patrols
                            .iter()
                            .copied()
                            .filter(|&p| tours[p].len() >= 2)
                            .collect();
                        if !agents.is_empty() {
                            let p = agents[self.rng.index(agents.len())];
                            let n = tours[p].len();
                            let len = 1 + self.rng.index(q.min(n));
                            let start = self.rng.index(n - len + 1);
                            for &si in &tours[p][start..start + len] {
                                mask[si] = true;
                            }
                        }
                    }
                }
                mask
            }
            fn coverage(&self, tours: &Tours) -> (u32, u32) {
                let mut seen = self.brands_ever;
                let mut claimed = 0;
                for &p in &self.patrols {
                    let (mut pos, mut used) = (self.apos[p], 0);
                    for &si in &tours[p] {
                        let s = &self.map.problem.spots[si];
                        if self.config.reachable_coverage {
                            let d = self.distance(pos, s.pos);
                            if d >= INF {
                                continue;
                            }
                            if used + d > self.day_steps {
                                break;
                            }
                            used += d;
                            pos = s.pos;
                        }
                        seen |= 1 << s.brand;
                        claimed |= 1 << s.brand;
                    }
                }
                (seen, claimed)
            }
            fn brand_value(brand: usize, seen: u32, claimed: u32) -> i32 {
                if seen & (1 << brand) == 0 {
                    100
                } else if claimed & (1 << brand) == 0 {
                    10
                } else {
                    1
                }
            }
            #[allow(clippy::too_many_arguments)] // 到達距離と系列の価値を元実装と同じ入力から評価する。
            fn insert_score(
                &self,
                si: usize,
                p: usize,
                j: usize,
                tours: &Tours,
                value: i32,
                seen: u32,
                claimed: u32,
            ) -> f64 {
                let prev = if j == 0 {
                    self.apos[p]
                } else {
                    self.map.problem.spots[tours[p][j - 1]].pos
                };
                let this = self.map.problem.spots[si].pos;
                let next = tours[p].get(j).map(|&x| self.map.problem.spots[x].pos);
                if !self.precomputed[prev] {
                    return -1e18;
                }
                let dprev = self.distance(prev, this);
                if dprev >= INF {
                    return -1e18;
                }
                let detour =
                    dprev + next.map_or(0, |n| self.distance(this, n) - self.distance(prev, n));
                let mut score = value as f64 * 10.0 - detour as f64;
                if self.config.ins_mode == 1 {
                    let mut pos = self.apos[p];
                    let mut distance = 0;
                    let mut arrival = if j == 0 { dprev } else { INF };
                    for (k, &x) in tours[p].iter().enumerate() {
                        let spot = &self.map.problem.spots[x];
                        distance = (distance + self.distance(pos, spot.pos)).min(INF);
                        pos = spot.pos;
                        if k + 1 == j {
                            arrival = distance + dprev;
                        }
                        if k >= j
                            && detour > 0
                            && distance <= self.day_steps
                            && distance > self.day_steps - detour
                        {
                            score -= Self::brand_value(spot.brand, seen, claimed) as f64 * 10.0;
                        }
                    }
                    if arrival > self.day_steps {
                        score -= value as f64 * 8.0;
                    }
                }
                score
            }
            fn insert_options(
                &self,
                si: usize,
                tours: &Tours,
                seen: u32,
                claimed: u32,
            ) -> [(f64, Option<(usize, usize)>); 2] {
                let mut best = [(-1e18, None), (-1e18, None)];
                let bv = Self::brand_value(self.map.problem.spots[si].brand, seen, claimed);
                for &p in &self.patrols {
                    for j in 0..=tours[p].len() {
                        let score = self.insert_score(si, p, j, tours, bv, seen, claimed);
                        if score > best[0].0 {
                            best[1] = best[0];
                            best[0] = (score, Some((p, j)));
                        } else if score > best[1].0 {
                            best[1] = (score, Some((p, j)));
                        }
                    }
                }
                best
            }
            fn repair(&mut self, tours: &mut Tours, mut removed: Vec<usize>, op: usize) {
                let (mut seen, mut claimed) = self.coverage(tours);
                if op != 1 {
                    removed.sort_unstable_by(|&a, &b| {
                        Self::brand_value(self.map.problem.spots[b].brand, seen, claimed).cmp(
                            &Self::brand_value(self.map.problem.spots[a].brand, seen, claimed),
                        )
                    });
                    for si in removed {
                        let opts = self.insert_options(si, tours, seen, claimed);
                        let choice = if op == 2 && opts[1].1.is_some() && self.rng.next() < 0.3 {
                            opts[1]
                        } else {
                            opts[0]
                        };
                        if let Some((p, j)) = choice.1 {
                            tours[p].insert(j, si);
                            let bit = 1 << self.map.problem.spots[si].brand;
                            seen |= bit;
                            claimed |= bit;
                        }
                    }
                } else {
                    while !removed.is_empty() {
                        let mut best = None;
                        let mut best_regret = -1e18;
                        for (i, &si) in removed.iter().enumerate() {
                            let opts = self.insert_options(si, tours, seen, claimed);
                            let Some((p, j)) = opts[0].1 else {
                                continue;
                            };
                            let regret = if opts[1].0 <= -1e17 {
                                f64::INFINITY
                            } else {
                                opts[0].0 - opts[1].0
                            };
                            if regret > best_regret {
                                best = Some((i, p, j));
                                best_regret = regret;
                            }
                        }
                        let Some((i, p, j)) = best else {
                            break;
                        };
                        let si = removed.remove(i);
                        tours[p].insert(j, si);
                        let bit = 1 << self.map.problem.spots[si].brand;
                        seen |= bit;
                        claimed |= bit;
                    }
                }
            }
            pub(super) fn destroy_repair(
                &mut self,
                tours: &mut Tours,
                d: usize,
                r: usize,
                q: usize,
                unified: bool,
            ) {
                let mask = self.destroy_mask(tours, d, q);
                let mut removed = vec![];
                let mut removed_mask = 0_u32;
                for &p in &self.patrols {
                    tours[p].retain(|&si| {
                        if mask[si] {
                            if removed_mask & (1 << si) == 0 {
                                removed_mask |= 1 << si;
                                removed.push(si);
                            }
                            false
                        } else {
                            true
                        }
                    });
                }
                if unified {
                    let mut used = removed_mask;
                    for &p in &self.patrols {
                        for &si in &tours[p] {
                            used |= 1 << si;
                        }
                    }
                    let mut unused: Vec<_> = (0..mask.len())
                        .filter(|&si| {
                            used & (1 << si) == 0 && self.map.problem.spots[si].stocks > 0
                        })
                        .collect();
                    for _ in 0..self.rng.index(3) {
                        if unused.is_empty() {
                            break;
                        }
                        let i = self.rng.index(unused.len());
                        removed.push(unused.remove(i));
                    }
                    if !removed.is_empty() && self.rng.next() < 0.1 {
                        let i = self.rng.index(removed.len());
                        removed.remove(i);
                    }
                }
                self.repair(tours, removed, r);
            }
            pub(super) fn shake(&mut self, base: &Tours) -> Tours {
                let mut tours = base.clone();
                let q = 2 + self
                    .rng
                    .index((self.map.problem.spots.len() / 2).clamp(3, 14));
                let d = self.rng.index(5);
                let r = self.rng.index(3);
                self.destroy_repair(&mut tours, d, r, q, false);
                tours
            }
            pub(super) fn tours_hash(&self, tours: &Tours) -> u64 {
                let mut h = 1469598103934665603_u64;
                for &p in &self.patrols {
                    for v in tours[p]
                        .iter()
                        .map(|&si| si as u64 + 1)
                        .chain(std::iter::once(0))
                    {
                        h ^= v
                            .wrapping_add(0x9e3779b97f4a7c15)
                            .wrapping_add(h << 6)
                            .wrapping_add(h >> 2);
                        h = h.wrapping_mul(1099511628211);
                    }
                }
                h
            }
        }
    }

    // 元ファイル: solver/paths.rs
    mod paths {
        use std::{borrow::Cow, cmp::Reverse, collections::BinaryHeap, sync::OnceLock};
        use tester::model::Problem;

        pub const INF: i32 = 1 << 29;

        #[derive(Clone)]
        pub struct MapInfo {
            pub problem: Problem,
            pub cells: Vec<u8>,
            pub spot_at: Vec<Option<usize>>,
            pub neighbors: Vec<[Option<usize>; 6]>,
            pub brands: usize,
        }

        impl MapInfo {
            pub fn new(problem: Problem) -> Self {
                let cells: Vec<_> = problem.map.cells.iter().flatten().copied().collect();
                let mut spot_at = vec![None; cells.len()];
                for (i, s) in problem.spots.iter().enumerate() {
                    spot_at[s.pos] = Some(i);
                }
                let neighbors = (0..cells.len())
                    .map(|p| std::array::from_fn(|d| problem.map.neighbor(p, d)))
                    .collect();
                let brands = problem.spots.iter().map(|s| s.brand + 1).max().unwrap_or(0);
                Self {
                    problem,
                    cells,
                    spot_at,
                    neighbors,
                    brands,
                }
            }
            pub fn fuel_cost(&self, p: usize) -> i32 {
                if self.cells[p] == 0 { 1 } else { 2 }
            }
            pub fn step_cost(&self, p: usize, road: &[u8]) -> i32 {
                match self.cells[p] {
                    0 => 2,
                    1 => [1, 2, 4][road[p] as usize],
                    2 => 3,
                    _ => INF,
                }
            }
        }

        #[derive(Clone, Debug)]
        pub struct Dij {
            pub dist: Vec<i32>,
            pub par: Vec<usize>,
            pub dir: Vec<i32>,
            source: usize,
            cache_index: Vec<u8>,
            cached_paths: Vec<OnceLock<Vec<i32>>>,
        }
        impl Dij {
            pub fn new(start: usize, map: &MapInfo, road: &[u8]) -> Self {
                Self::with_fuel(
                    start,
                    map,
                    road,
                    super::config::setting("PATH_FUEL", 0_i32).max(0),
                )
            }
            pub fn with_fuel(start: usize, map: &MapInfo, road: &[u8], fuel_weight: i32) -> Self {
                Self::variant(start, map, road, fuel_weight, 0)
            }
            pub fn variant(
                start: usize,
                map: &MapInfo,
                road: &[u8],
                fuel_weight: i32,
                salt: u32,
            ) -> Self {
                Self::variant_with_tie(
                    start,
                    map,
                    road,
                    fuel_weight,
                    salt,
                    super::config::setting("PATH_FUEL_TIE", 1) != 0,
                )
            }

            fn variant_with_tie(
                start: usize,
                map: &MapInfo,
                road: &[u8],
                fuel_weight: i32,
                salt: u32,
                prefer_fuel: bool,
            ) -> Self {
                Self::variant_with_options(
                    start,
                    map,
                    road,
                    fuel_weight,
                    salt,
                    prefer_fuel,
                    super::config::setting("PATH_CACHE", 1) != 0,
                )
            }

            fn variant_with_options(
                start: usize,
                map: &MapInfo,
                road: &[u8],
                fuel_weight: i32,
                salt: u32,
                prefer_fuel: bool,
                cache: bool,
            ) -> Self {
                let cache = cache && map.problem.spots.len() < u8::MAX as usize;
                let mut result = Self {
                    dist: vec![INF; map.cells.len()],
                    par: vec![usize::MAX; map.cells.len()],
                    dir: vec![-1; map.cells.len()],
                    source: start,
                    cache_index: if cache {
                        map.spot_at
                            .iter()
                            .map(|s| s.map_or(u8::MAX, |i| i as u8))
                            .collect()
                    } else {
                        vec![]
                    },
                    cached_paths: if cache {
                        (0..map.problem.spots.len())
                            .map(|_| OnceLock::new())
                            .collect()
                    } else {
                        vec![]
                    },
                };
                let mut queue = BinaryHeap::new();
                let mut weight = vec![(INF, INF); map.cells.len()];
                weight[start] = (0, 0);
                result.dist[start] = 0;
                queue.push(Reverse((0, 0, start)));
                while let Some(Reverse((distance, fuel, pos))) = queue.pop() {
                    if (distance, fuel) != weight[pos] {
                        continue;
                    }
                    let next_distance =
                        distance + map.step_cost(pos, road) * 4 + fuel_weight * map.fuel_cost(pos);
                    let next_fuel = fuel + if prefer_fuel { map.fuel_cost(pos) } else { 0 };
                    for (dir, next) in map.neighbors[pos].iter().enumerate() {
                        let Some(next) = *next else { continue };
                        let mut hash = salt
                            ^ (pos as u32).wrapping_mul(0x9e3779b9)
                            ^ (next as u32).wrapping_mul(0x85ebca6b);
                        hash = (hash ^ (hash >> 16)).wrapping_mul(0x7feb352d);
                        hash ^= hash >> 15;
                        let next_distance =
                            next_distance + if salt == 0 { 0 } else { (hash % 5) as i32 };
                        let next_weight = (next_distance, next_fuel);
                        if map.cells[next] == 3 || next_weight >= weight[next] {
                            continue;
                        }
                        weight[next] = next_weight;
                        result.dist[next] = result.dist[pos] + map.step_cost(pos, road);
                        result.par[next] = pos;
                        result.dir[next] = dir as i32;
                        queue.push(Reverse((next_distance, next_fuel, next)));
                    }
                }
                result
            }
            pub fn path(&self, start: usize, target: usize) -> Vec<i32> {
                self.path_view(start, target).into_owned()
            }

            pub fn path_view(&self, start: usize, target: usize) -> Cow<'_, [i32]> {
                if start == self.source
                    && let Some(&index) = self.cache_index.get(target)
                    && index != u8::MAX
                {
                    return Cow::Borrowed(
                        self.cached_paths[index as usize]
                            .get_or_init(|| self.build_path(start, target))
                            .as_slice(),
                    );
                }
                Cow::Owned(self.build_path(start, target))
            }

            fn build_path(&self, start: usize, target: usize) -> Vec<i32> {
                if self.dist[target] >= INF {
                    return vec![];
                }
                let mut pos = target;
                let mut path = Vec::new();
                while pos != start {
                    if self.par[pos] == usize::MAX {
                        return vec![];
                    }
                    path.push(self.dir[pos]);
                    pos = self.par[pos];
                }
                path.reverse();
                path
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use tester::generator::generate;

            #[test]
            fn spot_path_cache_matches_uncached_paths_and_does_not_alias_callers() {
                let map = MapInfo::new(generate(0, None).unwrap().problem);
                let start = map.problem.agents[0];
                for traffic in [false, true] {
                    let road: Vec<_> = (0..map.cells.len())
                        .map(|i| if traffic { (i % 3) as u8 } else { 0 })
                        .collect();
                    for weight in [0, 4] {
                        for salt in [0, 17] {
                            for prefer_fuel in [false, true] {
                                let reference = Dij::variant_with_options(
                                    start,
                                    &map,
                                    &road,
                                    weight,
                                    salt,
                                    prefer_fuel,
                                    false,
                                );
                                let cached = Dij::variant_with_options(
                                    start,
                                    &map,
                                    &road,
                                    weight,
                                    salt,
                                    prefer_fuel,
                                    true,
                                );
                                for target in 0..map.cells.len() {
                                    let expected = reference.path(start, target);
                                    assert_eq!(
                                        cached.path_view(start, target).as_ref(),
                                        expected.as_slice()
                                    );
                                    let mut returned = cached.path(start, target);
                                    assert_eq!(returned, expected);
                                    returned.push(99);
                                    assert_eq!(cached.path(start, target), expected);
                                    let parent = reference.par[target];
                                    if parent != usize::MAX {
                                        assert_eq!(
                                            cached.path(parent, target),
                                            reference.path(parent, target)
                                        );
                                    }
                                }
                                assert_eq!(cached.cached_paths.len(), map.problem.spots.len());
                                assert!(reference.cached_paths.is_empty());
                            }
                        }
                    }
                }
            }

            #[test]
            #[ignore = "経路キャッシュの実行時間を単独で測定する手動ベンチマーク"]
            fn benchmark_spot_path_cache() {
                use std::{hint::black_box, time::Instant};
                let map = MapInfo::new(generate(2, None).unwrap().problem);
                let road: Vec<_> = (0..map.cells.len()).map(|i| (i % 3) as u8).collect();
                let starts: Vec<_> = map
                    .problem
                    .agents
                    .iter()
                    .copied()
                    .chain(map.problem.spots.iter().map(|s| s.pos))
                    .collect();
                let calls = 1_000_000;
                for repeat in 0..3 {
                    for cache in if repeat % 2 == 0 {
                        [false, true]
                    } else {
                        [true, false]
                    } {
                        let paths: Vec<_> = starts
                            .iter()
                            .map(|&p| Dij::variant_with_options(p, &map, &road, 0, 0, false, cache))
                            .collect();
                        for (i, d) in paths.iter().enumerate() {
                            for spot in &map.problem.spots {
                                black_box(d.path(starts[i], spot.pos));
                            }
                        }
                        let begin = Instant::now();
                        for i in 0..calls {
                            let source = i % starts.len();
                            let target = map.problem.spots[(i * 17) % map.problem.spots.len()].pos;
                            black_box(paths[source].path_view(starts[source], target));
                        }
                        let elapsed = begin.elapsed().as_micros();
                        let bytes: usize = paths
                            .iter()
                            .map(|d| {
                                d.cache_index.len()
                                    + d.cached_paths.len()
                                        * std::mem::size_of::<OnceLock<Vec<i32>>>()
                                    + d.cached_paths
                                        .iter()
                                        .filter_map(|p| p.get())
                                        .map(|p| p.capacity() * 4)
                                        .sum::<usize>()
                            })
                            .sum();
                        eprintln!(
                            "path_cache repeat={repeat} cache={cache} calls={calls} micros={elapsed} extra_bytes={bytes}"
                        );
                    }
                }
            }

            #[test]
            fn fuel_tie_break_preserves_fastest_arrival_and_saves_fuel_on_equal_times() {
                let mut problem = generate(0, None).unwrap().problem;
                problem.map = tester::model::Map {
                    width: 3,
                    height: 2,
                    cells: vec![vec![0, 1, 3], vec![3, 0, 0]],
                };
                problem.agents = vec![0];
                problem.spots = vec![tester::model::Spot {
                    brand: 0,
                    pos: 5,
                    stocks: 1,
                }];
                problem.day_steps = vec![4, 4];
                problem.day_seconds = vec![60, 60];
                problem.validate().unwrap();
                let map = MapInfo::new(problem);
                let mut road = vec![0; map.cells.len()];
                road[1] = 1;
                // Both 0 -> 1 -> 5 and 0 -> 4 -> 5 arrive in four steps.
                // The busy road at cell 1 consumes two units, versus one at plain cell 4.
                for (prefer_fuel, expected_cost) in [(false, 3), (true, 2)] {
                    let d = Dij::variant_with_tie(0, &map, &road, 0, 0, prefer_fuel);
                    assert_eq!(d.dist[5], 4);
                    let mut game = tester::engine::Game::new(map.problem.clone(), &[0]);
                    // A first-day out-and-back trip leaves exactly one stay on road cell 1.
                    game.play_day(vec![vec![2, 5, -1]]).unwrap();
                    assert_eq!(game.traffic()[0].status, 1);
                    let fuel_before = game.agents[0].fuel;
                    game.play_day(vec![d.path(0, 5).into_iter().map(i64::from).collect()])
                        .unwrap();
                    assert_eq!(game.agents[0].pos, 5);
                    assert_eq!(game.agents[0].fuel, fuel_before - expected_cost);
                }
                road[1] = 0;
                let d = Dij::variant_with_tie(0, &map, &road, 0, 0, true);
                assert_eq!(d.dist[5], 3);
                assert_eq!(d.path(0, 5), vec![2, 3]);
            }

            #[test]
            fn weighted_variants_report_actual_duration_of_reconstructed_paths() {
                let map = MapInfo::new(generate(0, None).unwrap().problem);
                let road: Vec<_> = (0..map.cells.len()).map(|i| (i % 3) as u8).collect();
                for weight in [0, 4, 12] {
                    for salt in [0, 17, 255] {
                        for prefer_fuel in [false, true] {
                            let start = map.problem.agents[0];
                            let d = Dij::variant_with_tie(
                                start,
                                &map,
                                &road,
                                weight,
                                salt,
                                prefer_fuel,
                            );
                            for spot in &map.problem.spots {
                                let mut pos = start;
                                let mut duration = 0;
                                for direction in d.path(start, spot.pos) {
                                    duration += map.step_cost(pos, &road);
                                    pos = map.neighbors[pos][direction as usize].unwrap();
                                }
                                assert_eq!(pos, spot.pos);
                                assert_eq!(duration, d.dist[spot.pos]);
                            }
                        }
                    }
                }
            }
        }
    }

    // 元ファイル: solver/population.rs
    mod population {
        use super::alns::{Pool, Stats, Weights};
        use super::*;
        use std::collections::HashSet;

        #[derive(Clone)]
        struct Member {
            current_tours: Tours,
            best_tours: Tours,
            current: Sim,
            best: Sim,
            weights: Weights,
            scale: f64,
            iterations: u64,
        }

        impl Planner {
            fn branch(&mut self, parent: &Tours, ordinal: usize) -> Tours {
                let mut tours = parent.clone();
                for _ in 0..self.config.pop_branch + ordinal % 3 {
                    if self.config.neighbor_mode == 1 || self.rng.next() < 0.55 {
                        let d = self.rng.index(5);
                        let r = self.rng.index(3);
                        let q = self.unified_degree();
                        self.destroy_repair(&mut tours, d, r, q, true);
                    } else if self.config.restart_mode != 0 {
                        tours = self.shake(&tours);
                    } else {
                        self.mutate(&mut tours, true);
                    }
                }
                tours
            }
            fn diversify(&mut self, tours: &mut Tours, hashes: &HashSet<u64>, stats: &mut Stats) {
                for _ in 0..self.config.retry + 4 {
                    if !hashes.contains(&self.tours_hash(tours)) {
                        break;
                    }
                    stats.memo_hits += 1;
                    self.retry_tours(tours);
                }
            }
            fn member(
                &mut self,
                tours: Tours,
                ordinal: usize,
                width: usize,
                pool: &mut Pool,
            ) -> Member {
                let mut sim = self.simulate(&tours);
                self.rollout(&mut sim);
                pool.add(self, &sim, &tours);
                Member {
                    current_tours: tours.clone(),
                    best_tours: tours,
                    current: sim.clone(),
                    best: sim,
                    weights: Weights::default(),
                    scale: 0.75
                        + 0.5
                            * if width <= 1 {
                                0.5
                            } else {
                                ordinal as f64 / (width - 1) as f64
                            },
                    iterations: 0,
                }
            }
            #[allow(clippy::too_many_arguments)] // 通常探索と共有する予算・最良解・候補プール・統計。
            pub(super) fn run_population(
                &mut self,
                init: &Tours,
                start: Instant,
                mut deadline: u64,
                best: &mut Sim,
                best_tours: &mut Tours,
                pool: &mut Pool,
                stats: &mut Stats,
            ) {
                let width = if self.config.pop_width > 0 {
                    self.config.pop_width
                } else if self.config.budget_ms >= 20_000 && self.config.iterations == 0 {
                    6
                } else {
                    4
                }
                .clamp(1, 12);
                let mut hashes = HashSet::new();
                let mut population = vec![];
                for i in 0..width {
                    let mut tours = if i == 0 {
                        init.clone()
                    } else if self.config.session_mode == 0 && i % 3 == 0 {
                        self.greedy_init(true)
                    } else {
                        self.branch(init, i)
                    };
                    if i != 0 {
                        self.diversify(&mut tours, &hashes, stats);
                    }
                    let member = self.member(tours, i, width, pool);
                    hashes.insert(self.tours_hash(&member.current_tours));
                    hashes.insert(self.tours_hash(&member.best_tours));
                    if self.better(&member.best, best) {
                        *best = member.best.clone();
                        *best_tours = member.best_tours.clone();
                    }
                    population.push(member);
                }
                let interval = self.config.pop_cull_iters.max(width as u64 * 40);
                let mut next_cull = interval;
                let mut last_cull = Self::elapsed(start);
                let mut extended = false;
                loop {
                    let mut now = Self::elapsed(start);
                    if self.expired() {
                        break;
                    }
                    if self.config.iterations > 0 {
                        if stats.iterations >= self.config.iterations {
                            break;
                        }
                    } else if now >= deadline {
                        if pool.active && pool.entries.len() < 2 && !extended {
                            extended = true;
                            deadline = self.config.budget_ms;
                            now = Self::elapsed(start);
                            if now >= deadline {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    let index = stats.iterations as usize % width;
                    stats.iterations += 1;
                    let member = &mut population[index];
                    member.iterations += 1;
                    let (mut tours, operators) =
                        self.propose(&member.current_tours, &member.best_tours, &member.weights);
                    let mut hash = self.tours_hash(&tours);
                    let mut duplicate = hashes.contains(&hash);
                    for _ in 0..self.config.retry {
                        if !duplicate {
                            break;
                        }
                        stats.memo_hits += 1;
                        self.retry_tours(&mut tours);
                        hash = self.tours_hash(&tours);
                        duplicate = hashes.contains(&hash);
                    }
                    if duplicate {
                        stats.memo_hits += 1;
                        member.weights.record(operators, 0.0);
                    } else {
                        hashes.insert(hash);
                        let mut sim = self.simulate(&tours);
                        stats.evaluations += 1;
                        let lazy = self.config.lazy
                            && self.config.depth > 0
                            && sim.newkinds < member.best.newkinds;
                        if lazy {
                            stats.lazy_skips += 1;
                        } else {
                            self.rollout(&mut sim);
                            pool.add(self, &sim, &tours);
                        }
                        let improved = self.better(&sim, &member.best);
                        member
                            .weights
                            .record(operators, self.reward(&sim, &member.best));
                        let fraction = if self.config.iterations > 0 {
                            stats.iterations as f64 / self.config.iterations as f64
                        } else {
                            now as f64 / deadline.max(1) as f64
                        };
                        let accept = self.accept_sa(&sim, &member.current, fraction, member.scale);
                        if improved {
                            member.best = sim.clone();
                            member.best_tours = tours.clone();
                            if self.better(&member.best, best) {
                                *best = member.best.clone();
                                *best_tours = member.best_tours.clone();
                            }
                        }
                        if accept {
                            member.current = sim;
                            member.current_tours = tours;
                        }
                    }
                    if member.iterations % 60 == 0 {
                        member.weights.update();
                    }
                    let after = Self::elapsed(start);
                    if stats.iterations >= next_cull
                        || (self.config.pop_cull_ms > 0
                            && after - last_cull >= self.config.pop_cull_ms)
                    {
                        if width > 1 {
                            population.sort_by(|a, b| {
                                self.key(&b.best)
                                    .cmp(&self.key(&a.best))
                                    .then(Self::scalar(&b.best).total_cmp(&Self::scalar(&a.best)))
                                    .then(
                                        self.tours_hash(&a.best_tours)
                                            .cmp(&self.tours_hash(&b.best_tours)),
                                    )
                            });
                            let elite = if self.config.pop_elite > 0 {
                                self.config.pop_elite
                            } else {
                                width.div_ceil(2)
                            }
                            .clamp(1, width - 1);
                            population.truncate(elite);
                            for slot in elite..width {
                                let mut tours = self
                                    .branch(&population[(slot - elite) % elite].best_tours, slot);
                                self.diversify(&mut tours, &hashes, stats);
                                let child = self.member(
                                    tours,
                                    (slot + stats.culls as usize) % width,
                                    width,
                                    pool,
                                );
                                hashes.insert(self.tours_hash(&child.current_tours));
                                hashes.insert(self.tours_hash(&child.best_tours));
                                if self.better(&child.best, best) {
                                    *best = child.best.clone();
                                    *best_tours = child.best_tours.clone();
                                }
                                population.push(child);
                            }
                            stats.culls += 1;
                        }
                        next_cull = stats.iterations + interval;
                        last_cull = Self::elapsed(start);
                    }
                    if stats.iterations >= 50_000_000 {
                        break;
                    }
                }
                for member in population {
                    if self.better(&member.best, best) {
                        *best = member.best;
                        *best_tours = member.best_tours;
                    }
                }
            }
        }
    }

    // 元ファイル: solver/repair.rs
    mod repair {
        //! 補給車の軌跡を固定し、巡回車1台の時刻・位置・燃料・獲得集合を探索する。
        use super::*;
        use std::collections::HashMap;

        #[derive(Clone, Copy)]
        struct Label {
            pos: usize,
            mask: u32,
            fuel: i32,
            parent: usize,
            action: i32,
        }

        impl Planner {
            pub(super) fn repair_full_day(
                &mut self,
                original: &Plan,
                budget_ms: u64,
            ) -> Option<Plan> {
                let started = Instant::now();
                let steps = self.day_steps as usize;
                let n = self.map.cells.len();
                let full = 1_u32.checked_shl(self.map.brands as u32)?.wrapping_sub(1);
                let actual = self.replay(original, &self.apos, &self.afuel)?;
                if actual.claimed == full {
                    return Some(original.clone());
                }
                let mut positions = vec![vec![0; steps + 1]; self.na];
                for a in 0..self.na {
                    let mut t = 0;
                    let mut pos = self.apos[a];
                    positions[a][0] = pos;
                    for &op in &original[a] {
                        let duration = if op < 0 { -op } else { self.step_cost[pos] } as usize;
                        positions[a][t + 1..=t + duration].fill(pos);
                        if op >= 0 {
                            pos = self.map.neighbors[pos][op as usize]?;
                            positions[a][t + duration] = pos;
                        }
                        t += duration;
                    }
                }
                let mut best: Option<(f64, Plan)> = None;
                for (patrol_index, &p) in self.patrols.iter().enumerate() {
                    if self.expired() || Self::elapsed(started) >= budget_ms {
                        break;
                    }
                    let other = self
                        .patrols
                        .iter()
                        .filter(|&&a| a != p)
                        .fold(0u32, |mask, &a| {
                            positions[a][1..].iter().fold(mask, |m, &pos| {
                                m | self.map.spot_at[pos]
                                    .map_or(0, |s| 1 << self.map.problem.spots[s].brand)
                            })
                        });
                    let required = full & !other;
                    if required.count_ones() > 10 {
                        continue;
                    }
                    let cell_mask: Vec<u32> = (0..n)
                        .map(|pos| {
                            self.map.spot_at[pos].map_or(0, |s| {
                                if self.map.problem.spots[s].stocks > 0 {
                                    (1 << self.map.problem.spots[s].brand) & required
                                } else {
                                    0
                                }
                            })
                        })
                        .collect();
                    let mut labels = vec![Label {
                        pos: self.apos[p],
                        mask: 0,
                        fuel: self.afuel[p],
                        parent: usize::MAX,
                        action: -1,
                    }];
                    let mut layers = vec![HashMap::<(usize, u32), usize>::new(); steps + 1];
                    layers[0].insert((self.apos[p], 0), 0);
                    let now = Self::elapsed(started);
                    let per_until = now
                        + budget_ms.saturating_sub(now)
                            / (self.patrols.len() - patrol_index) as u64;
                    let mut stopped = false;
                    for t in 0..steps {
                        if self.expired()
                            || Self::elapsed(started) >= per_until
                            || labels.len() > 400_000
                        {
                            stopped = true;
                            break;
                        }
                        let current = std::mem::take(&mut layers[t]);
                        for (_, id) in current {
                            let node = labels[id];
                            for action in -1..6 {
                                let duration = if action < 0 {
                                    1
                                } else {
                                    self.step_cost[node.pos]
                                } as usize;
                                if t + duration > steps {
                                    continue;
                                }
                                let dest = if action < 0 {
                                    node.pos
                                } else {
                                    let Some(q) = self.map.neighbors[node.pos][action as usize]
                                    else {
                                        continue;
                                    };
                                    if self.map.cells[q] == 3 {
                                        continue;
                                    }
                                    q
                                };
                                let mut fuel = node.fuel;
                                let mut mask = node.mask;
                                for when in t + 1..=t + duration {
                                    let here = if when == t + duration { dest } else { node.pos };
                                    if when == t + duration && action >= 0 {
                                        fuel -= self.fuel_cost[node.pos];
                                    }
                                    if fuel < 0 {
                                        break;
                                    }
                                    mask |= cell_mask[here];
                                    if self.supplies.iter().any(|&s| positions[s][when] == here) {
                                        fuel = self.map.problem.fuel_limits as i32;
                                    }
                                }
                                if fuel < 0 {
                                    continue;
                                }
                                let layer = &mut layers[t + duration];
                                let key = (dest, mask);
                                if layer.get(&key).is_some_and(|&j| labels[j].fuel >= fuel) {
                                    continue;
                                }
                                layer.insert(key, labels.len());
                                labels.push(Label {
                                    pos: dest,
                                    mask,
                                    fuel,
                                    parent: id,
                                    action,
                                });
                            }
                        }
                    }
                    if stopped {
                        continue;
                    }
                    let mut endings: Vec<_> = layers[steps]
                        .iter()
                        .filter_map(|(&(pos, mask), &id)| {
                            if mask != required {
                                return None;
                            }
                            let dist = self.distances[actual.pos[p]]
                                .as_ref()
                                .map_or(0, |d| d.dist[pos]);
                            Some((labels[id].fuel as f64 * 0.1 - dist as f64, id))
                        })
                        .collect();
                    endings.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
                    for &(value, mut id) in endings.iter().take(8) {
                        let mut ops = Vec::new();
                        while labels[id].parent != usize::MAX {
                            ops.push(labels[id].action);
                            id = labels[id].parent;
                        }
                        ops.reverse();
                        let mut compressed = Vec::new();
                        for op in ops {
                            if op == -1 && compressed.last().is_some_and(|&x| x < 0) {
                                *compressed.last_mut().unwrap() -= 1;
                            } else {
                                compressed.push(op);
                            }
                        }
                        let mut plan = original.clone();
                        plan[p] = compressed;
                        if self
                            .replay(&plan, &self.apos, &self.afuel)
                            .is_some_and(|r| r.claimed == full)
                            && best.as_ref().is_none_or(|b| value > b.0)
                        {
                            best = Some((value, plan));
                        }
                    }
                }
                best.map(|(_, plan)| plan)
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use tester::engine::Game;
            use tester::model::{Map, Spot};

            #[test]
            fn replacement_collects_missing_brand_using_midroute_supply() {
                let problem = Problem {
                    starts_at: 0,
                    day_seconds: vec![60],
                    day_steps: vec![8],
                    map: Map {
                        height: 1,
                        width: 5,
                        cells: vec![vec![0; 5]],
                    },
                    spots: vec![
                        Spot {
                            brand: 0,
                            pos: 0,
                            stocks: 1,
                        },
                        Spot {
                            brand: 1,
                            pos: 4,
                            stocks: 1,
                        },
                    ],
                    agents: vec![0, 2],
                    fuel_limits: 3,
                    players: 1,
                    busy_threshold: 1,
                    jammed_threshold: 3,
                };
                let mut game = Game::new(problem.clone(), &[0, 1]);
                let mut planner = Planner::new(problem, 0);
                planner.prepare(&game.observation(0), 8);
                let original = vec![vec![-8], vec![-8]];
                let plan = planner
                    .repair_full_day(&original, 1000)
                    .expect("補給地点経由で全種類を獲得できる");
                game.play_day(
                    plan.iter()
                        .map(|a| a.iter().map(|&x| i64::from(x)).collect())
                        .collect(),
                )
                .unwrap();
                assert_eq!(game.score.daily_brands, 2);
                assert_eq!(game.agents[0].pos, 4);
                assert_eq!(game.agents[0].fuel, 1);
            }
            #[test]
            #[ignore = "保存リプレイを用いた明示的な修復実験"]
            fn repair_saved_replay() {
                let path = std::env::var("PROCON37_REPAIR_REPLAY").unwrap();
                let record: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
                let problem: Problem =
                    serde_json::from_value(record["case"]["problem"].clone()).unwrap();
                let kinds: Vec<u8> = serde_json::from_value(record["kinds"].clone()).unwrap();
                let mut game = Game::new(problem.clone(), &kinds);
                let mut planner = Planner::new(problem, 0);
                for d in record["days"].as_array().unwrap() {
                    let plan: Plan = serde_json::from_value(d["plan"].clone()).unwrap();
                    planner.prepare(
                        &game.observation(0),
                        game.problem.day_steps[game.day] as i32,
                    );
                    let old = planner
                        .replay(&plan, &planner.apos, &planner.afuel)
                        .unwrap();
                    if old.claimed.count_ones() < planner.map.brands as u32 {
                        let repaired = planner.repair_full_day(&plan, 20_000);
                        eprintln!("day={} repaired={}", game.day, repaired.is_some());
                        if let Some(plan) = repaired {
                            game.play_day(
                                plan.iter()
                                    .map(|a| a.iter().map(|&x| i64::from(x)).collect())
                                    .collect(),
                            )
                            .unwrap();
                            let out = std::env::var("PROCON37_REPAIR_OUTPUT").unwrap();
                            std::fs::write(out,serde_json::json!({"day":game.day-1,"plan":plan,"agents":game.agents,"daily":game.score.daily_brands}).to_string()).unwrap();
                        }
                        break;
                    }
                    game.play_day(
                        plan.iter()
                            .map(|a| a.iter().map(|&x| i64::from(x)).collect())
                            .collect(),
                    )
                    .unwrap();
                }
            }
        }
    }

    // 元ファイル: solver/replay.rs
    mod replay {
        use super::*;

        #[derive(Debug)]
        pub(super) struct Replay {
            pub claimed: u32,
            pub balls: i32,
            pub pos: Vec<usize>,
            pub fuel: Vec<i32>,
            pub stay: Vec<i32>,
            pub shortage: i32,
        }

        #[derive(Clone, Copy, Default)]
        struct Pending {
            destination: Option<usize>,
            cost: i32,
            done: i32,
        }

        impl Planner {
            /// 行動予約→移動反映→在庫獲得→補給の順序を公式と揃える。
            pub(super) fn replay(
                &self,
                ops: &Plan,
                initial_pos: &[usize],
                initial_fuel: &[i32],
            ) -> Option<Replay> {
                self.replay_with_fuel(ops, initial_pos, initial_fuel, false)
            }

            pub(super) fn replay_with_fuel(
                &self,
                ops: &Plan,
                initial_pos: &[usize],
                initial_fuel: &[i32],
                soft_fuel: bool,
            ) -> Option<Replay> {
                let n = initial_pos.len();
                if ops.len() != n
                    || initial_fuel.len() != n
                    || self.kinds.len() != n
                    || self.day_steps <= 0
                {
                    return None;
                }
                let mut result = Replay {
                    claimed: 0,
                    balls: 0,
                    pos: initial_pos.to_vec(),
                    fuel: initial_fuel.to_vec(),
                    stay: vec![0; self.map.cells.len()],
                    shortage: 0,
                };
                let mut next_op = vec![0; n];
                let mut pending = vec![Pending::default(); n];
                let reserve = |a: usize,
                               time: i32,
                               result: &Replay,
                               next_op: &mut [usize],
                               pending: &mut [Pending]|
                 -> Option<()> {
                    let &op = ops[a].get(next_op[a])?;
                    next_op[a] += 1;
                    if op < 0 {
                        let duration = op.checked_neg()?;
                        if duration > self.day_steps - time {
                            return None;
                        }
                        pending[a] = Pending {
                            destination: None,
                            cost: 0,
                            done: time + duration,
                        };
                    } else {
                        if op >= 6 {
                            return None;
                        }
                        let from = result.pos[a];
                        let next = self.map.neighbors[from][op as usize]?;
                        if self.map.cells[next] == 3 {
                            return None;
                        }
                        let duration = self.step_cost[from];
                        let cost = if self.kinds[a] == 0 {
                            self.fuel_cost[from]
                        } else {
                            0
                        };
                        if duration > self.day_steps - time {
                            return None;
                        }
                        pending[a] = Pending {
                            destination: Some(next),
                            cost,
                            done: time + duration,
                        };
                    }
                    Some(())
                };
                for a in 0..n {
                    reserve(a, 0, &result, &mut next_op, &mut pending)?;
                }
                let mut stock: Vec<_> = self.map.problem.spots.iter().map(|s| s.stocks).collect();
                let mut taken = vec![0_u32; n];
                for time in 1..=self.day_steps {
                    for (a, action) in pending.iter().enumerate() {
                        if action.done == time
                            && let Some(dest) = action.destination
                        {
                            // A pending move may receive supply before completion.
                            // Fuel is charged before this step's movement and refill.
                            if result.fuel[a] < action.cost {
                                if !soft_fuel {
                                    return None;
                                }
                                result.shortage += action.cost - result.fuel[a].max(0);
                            }
                            result.fuel[a] -= action.cost;
                            result.pos[a] = dest;
                        }
                    }
                    for (p, taken) in taken.iter_mut().enumerate() {
                        if self.kinds[p] != 0 {
                            continue;
                        }
                        let Some(si) = self.map.spot_at[result.pos[p]] else {
                            continue;
                        };
                        if *taken & (1 << si) != 0 || stock[si] == 0 {
                            continue;
                        }
                        *taken |= 1 << si;
                        stock[si] -= 1;
                        result.claimed |= 1 << self.map.problem.spots[si].brand;
                        result.balls += 1;
                    }
                    for p in 0..n {
                        if self.kinds[p] == 0
                            && (0..n).any(|s| self.kinds[s] == 1 && result.pos[s] == result.pos[p])
                        {
                            result.fuel[p] = self.map.problem.fuel_limits as i32;
                        }
                    }
                    for &pos in &result.pos {
                        if self.map.cells[pos] == 1 {
                            result.stay[pos] += 1;
                        }
                    }
                    if time < self.day_steps {
                        for a in 0..n {
                            if pending[a].done == time {
                                reserve(a, time, &result, &mut next_op, &mut pending)?;
                            }
                        }
                    }
                }
                if (0..n).any(|a| pending[a].done != self.day_steps || next_op[a] != ops[a].len()) {
                    return None;
                }
                Some(result)
            }
        }
    }

    // 元ファイル: solver/supply.rs
    mod supply {
        use super::*;

        impl Planner {
            pub(super) fn configure_supply(&mut self, targets: &[Option<usize>]) {
                self.supply_target = vec![None; self.na];
                self.optimistic = vec![false; self.na];
                self.rdv_supply = vec![None; self.na];
                for (k, &sup) in self.supplies.iter().enumerate() {
                    if let Some(p) = targets[k] {
                        self.supply_target[sup] = Some(p);
                        if self.apos[sup] == self.apos[p] {
                            self.optimistic[p] = true;
                        }
                    }
                }
                for (k, sup) in self.supplies.clone().into_iter().enumerate() {
                    let Some(p) = targets[k] else {
                        continue;
                    };
                    if self.apos[sup] == self.apos[p] {
                        continue;
                    }
                    let d = self.dij(self.apos[sup]);
                    self.precomputed[self.apos[sup]] = true;
                    if self.rdv_supply[p].is_none_or(|prev| {
                        d.dist[self.apos[p]] < self.distance(self.apos[prev], self.apos[p])
                    }) {
                        self.rdv_supply[p] = Some(sup);
                    }
                }
                self.order = self.patrols.clone();
                self.order
                    .sort_by_key(|&p| std::cmp::Reverse(self.afuel[p]));
                self.order.sort_by_key(|&p| !self.optimistic[p]);
            }

            fn supply_target_options(&mut self) -> Vec<Vec<Option<usize>>> {
                let mut options = vec![];
                if self.supplies.is_empty() || self.patrols.is_empty() {
                    options.push(vec![None; self.supplies.len()]);
                } else {
                    options.push(
                        (0..self.supplies.len())
                            .map(|k| Some(self.patrols[k % self.patrols.len()]))
                            .collect(),
                    );
                    let mut by_fuel = self.patrols.clone();
                    by_fuel.sort_by_key(|&p| self.afuel[p]);
                    options.push(
                        (0..self.supplies.len())
                            .map(|k| Some(by_fuel[k % by_fuel.len()]))
                            .collect(),
                    );
                    let mut nearest = vec![];
                    for sup in self.supplies.clone() {
                        let d = self.dij(self.apos[sup]);
                        nearest.push(
                            self.patrols
                                .iter()
                                .copied()
                                .min_by_key(|&p| d.dist[self.apos[p]]),
                        );
                    }
                    options.push(nearest);
                    if self.supplies.len() == 1 {
                        for &p in &self.patrols {
                            options.push(vec![Some(p)]);
                        }
                    }
                    // A seven-agent fleet has at most 64 assignments for its usual
                    // two or three supply vehicles. Keep larger custom fleets bounded.
                    if self.config.enumerate_supply_targets
                        && self.supplies.len() > 1
                        && let Some(count) =
                            self.patrols.len().checked_pow(self.supplies.len() as u32)
                        && count <= 64
                    {
                        for mut code in 0..count {
                            let mut targets = Vec::with_capacity(self.supplies.len());
                            for _ in &self.supplies {
                                targets.push(Some(self.patrols[code % self.patrols.len()]));
                                code /= self.patrols.len();
                            }
                            options.push(targets);
                        }
                    }
                }
                options.sort();
                options.dedup();
                options
            }

            pub(super) fn select_supply_targets(&mut self) -> Vec<Option<usize>> {
                let options = self.supply_target_options();
                let mut best = None;
                let mut targets = options[0].clone();
                for candidate in options {
                    if self.config.enumerate_supply_targets && best.is_some() && self.expired() {
                        break;
                    }
                    self.configure_supply(&candidate);
                    let tours = self.greedy_init(false);
                    let mut sim = self.simulate(&tours);
                    self.rollout(&mut sim);
                    if best.as_ref().is_none_or(|b| self.better(&sim, b)) {
                        best = Some(sim);
                        targets = candidate;
                    }
                }
                targets
            }

            fn plan_supply(&mut self, start: usize, follow: Option<&[usize]>) -> Vec<i32> {
                let (mut pos, mut used, mut wait) = (start, 0, 0);
                let mut plan = vec![];
                while used < self.day_steps {
                    let target = follow
                        .filter(|f| !f.is_empty())
                        .map(|f| f[(used as usize).min(f.len() - 1)]);
                    let cost = self.step_cost[pos];
                    if target.is_none_or(|t| t == pos) || used + cost > self.day_steps {
                        wait += 1;
                        used += 1;
                        continue;
                    }
                    let dirs = self.dij(pos).path(pos, target.unwrap());
                    if dirs.is_empty() {
                        wait += 1;
                        used += 1;
                        continue;
                    }
                    if wait > 0 {
                        plan.push(-wait);
                        wait = 0;
                    }
                    plan.push(dirs[0]);
                    pos = self.map.neighbors[pos][dirs[0] as usize].unwrap();
                    used += cost;
                }
                if wait > 0 {
                    plan.push(-wait);
                }
                if plan.is_empty() {
                    plan.push(-self.day_steps);
                }
                plan
            }

            fn supply_walk(
                &mut self,
                plan: &mut Vec<i32>,
                pos: &mut usize,
                time: &mut i32,
                wait: &mut i32,
                target: usize,
            ) {
                if *pos == target {
                    return;
                }
                let dirs = self.dij(*pos).path(*pos, target);
                if *wait > 0 {
                    plan.push(-*wait);
                    *wait = 0;
                }
                for dir in dirs {
                    plan.push(dir);
                    *time += self.step_cost[*pos];
                    *pos = self.map.neighbors[*pos][dir as usize].unwrap();
                }
            }

            pub(super) fn fill_supply(&mut self, s: &mut Sim) {
                for (k, sup) in self.supplies.clone().into_iter().enumerate() {
                    let target = self.supply_target[sup];
                    if self.config.supply_mode == 0 {
                        if let Some(p) = target {
                            if self.apos[sup] == self.apos[p] {
                                continue;
                            }
                            if self.rdv_supply[p] == Some(sup) {
                                if let Some(resume) = s.rdv_resume[p] {
                                    let cell = s.pbs[p][resume as usize];
                                    let d = self.dij(self.apos[sup]);
                                    let mut plan = d.path(self.apos[sup], cell);
                                    if resume > d.dist[cell] {
                                        plan.push(-(resume - d.dist[cell]));
                                    }
                                    plan.extend_from_slice(&s.ops[p][s.rdv_prefix[p]..]);
                                    if plan.is_empty() {
                                        plan.push(-self.day_steps.max(1));
                                    }
                                    s.ops[sup] = plan;
                                    continue;
                                }
                                let d = self.dij(self.apos[sup]);
                                let end = s.endpos[p];
                                if d.dist[end] <= self.day_steps {
                                    let mut plan = d.path(self.apos[sup], end);
                                    if d.dist[end] < self.day_steps {
                                        plan.push(-(self.day_steps - d.dist[end]));
                                    }
                                    if plan.is_empty() {
                                        plan.push(-self.day_steps.max(1));
                                    }
                                    s.ops[sup] = plan;
                                    continue;
                                }
                            }
                        }
                        s.ops[sup] =
                            self.plan_supply(self.apos[sup], target.map(|p| s.pbs[p].as_slice()));
                        continue;
                    }
                    let mirrored = self.config.supply_mode == 2
                        && s.sup_mirror[k].is_some()
                        && s.sup_detach[k].is_some();
                    if !mirrored && target.is_some_and(|p| self.apos[sup] == self.apos[p]) {
                        continue;
                    }
                    if !mirrored && s.sup_events[k].is_empty() && s.sup_overnight[k].is_none() {
                        s.ops[sup] =
                            self.plan_supply(self.apos[sup], target.map(|p| s.pbs[p].as_slice()));
                        continue;
                    }
                    let (mut plan, mut pos, mut time, mut wait) = (vec![], self.apos[sup], 0, 0);
                    let mut detach = -1;
                    if mirrored {
                        let p = s.sup_mirror[k].unwrap();
                        detach = s.sup_detach[k].unwrap();
                        let mut mirror_from = 0;
                        if self.apos[sup] != self.apos[p] {
                            let (cell, resume, _) = s.sup_events[k][0];
                            self.supply_walk(&mut plan, &mut pos, &mut time, &mut wait, cell);
                            if resume > time {
                                wait += resume - time;
                                time = resume;
                            }
                            mirror_from = s.rdv_prefix[p];
                        }
                        for &op in &s.ops[p][mirror_from..] {
                            if time >= detach {
                                break;
                            }
                            if op >= 0 {
                                let cost = self.step_cost[pos];
                                if time + cost > detach {
                                    break;
                                }
                                if wait > 0 {
                                    plan.push(-wait);
                                    wait = 0;
                                }
                                plan.push(op);
                                time += cost;
                                pos = self.map.neighbors[pos][op as usize].unwrap();
                            } else {
                                let w = (-op).min(detach - time);
                                wait += w;
                                time += w;
                            }
                        }
                    }
                    for &(cell, resume, _) in &s.sup_events[k] {
                        if mirrored && resume <= detach {
                            continue;
                        }
                        self.supply_walk(&mut plan, &mut pos, &mut time, &mut wait, cell);
                        if resume > time {
                            wait += resume - time;
                            time = resume;
                        }
                    }
                    if let Some(p) = s.sup_overnight[k] {
                        self.supply_walk(&mut plan, &mut pos, &mut time, &mut wait, s.endpos[p]);
                    }
                    if time < self.day_steps {
                        wait += self.day_steps - time;
                    }
                    if wait > 0 {
                        plan.push(-wait);
                    }
                    if plan.is_empty() {
                        plan.push(-self.day_steps.max(1));
                    }
                    s.ops[sup] = plan;
                }
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use tester::{engine::Game, generator::generate};

            #[test]
            fn enumerated_supply_assignments_agree_with_independent_replays() {
                for (seed, kinds, expected_count) in [
                    (4, vec![0, 1, 0, 1, 0], 9),
                    (2, vec![0, 1, 0, 1, 0, 1, 0], 64),
                ] {
                    let problem = generate(seed, None).unwrap().problem;
                    let mut game = Game::new(problem.clone(), &kinds);
                    if seed == 2 {
                        // A later-day observation with little fuel exercises rendezvous
                        // and competing supply vehicles as well as ordinary following.
                        game.day = 1;
                        for (a, agent) in game.agents.iter_mut().enumerate() {
                            if agent.kind == 0 {
                                agent.fuel = (a as u32 + 1).min(agent.fuel);
                            }
                        }
                    }
                    let mut planner = Planner::new(problem, 0);
                    planner.config.depth = 2;
                    planner.config.supply_mode = 2;
                    for _ in 0..3 {
                        planner.prepare(
                            &game.observation(0),
                            game.problem.day_steps[game.day] as i32,
                        );
                        planner.config.enumerate_supply_targets = false;
                        let original = planner.supply_target_options();
                        planner.config.enumerate_supply_targets = true;
                        let options = planner.supply_target_options();
                        assert_eq!(options.len(), expected_count);
                        assert!(original.iter().all(|targets| options.contains(targets)));
                        for targets in options {
                            planner.configure_supply(&targets);
                            let tours = planner.greedy_init(false);
                            let mut sim = planner.simulate(&tours);
                            planner.fill_supply(&mut sim);
                            let actual = planner
                                .replay(&sim.ops, &planner.apos, &planner.afuel)
                                .expect("enumerated supply assignment must produce a legal plan");
                            let mut checked = game.clone();
                            checked
                                .play_day(
                                    sim.ops
                                        .iter()
                                        .map(|ops| ops.iter().map(|&op| i64::from(op)).collect())
                                        .collect(),
                                )
                                .unwrap();
                            assert_eq!(actual.claimed, sim.claimed);
                            assert_eq!(actual.balls, sim.balls);
                            assert_eq!(
                                actual.claimed.count_ones() as usize,
                                checked.score.daily_brands - game.score.daily_brands
                            );
                            assert_eq!(actual.balls as u32, checked.score.udon - game.score.udon);
                            assert_eq!(
                                actual.pos,
                                checked.agents.iter().map(|a| a.pos).collect::<Vec<_>>()
                            );
                            assert_eq!(
                                actual.fuel,
                                checked
                                    .agents
                                    .iter()
                                    .map(|a| a.fuel as i32)
                                    .collect::<Vec<_>>()
                            );
                        }
                        let selected = planner.select_supply_targets();
                        planner.configure_supply(&selected);
                        let tours = planner.greedy_init(false);
                        let mut sim = planner.simulate(&tours);
                        planner.fill_supply(&mut sim);
                        game.play_day(
                            sim.ops
                                .iter()
                                .map(|ops| ops.iter().map(|&op| i64::from(op)).collect())
                                .collect(),
                        )
                        .unwrap();
                        planner.brands_ever |= sim.claimed;
                    }
                }
            }
        }
    }

    // 元ファイル: solver/target_search.rs
    mod target_search {
        //! 補給先の初期選択ごとに経路を作り、実際の翌日を再生して比較する。
        use super::*;
        use tester::model::Agent;

        impl Planner {
            pub(super) fn plan_day_target_search(
                &mut self,
                observation: &Observation,
                steps: i32,
            ) -> Plan {
                self.prepare(observation, steps);
                let started = Instant::now();
                let default = self.select_supply_targets();
                let mut options = vec![default];
                for &p in &self.patrols {
                    let target = vec![Some(p)];
                    if !options.contains(&target) {
                        options.push(target);
                    }
                }
                let future_days = config::setting("TARGET_DEPTH", 1_usize)
                    .min(self.map.problem.day_steps.len() - observation.day - 1);
                let per_target = (self.config.budget_ms / options.len().max(1) as u64).max(1);
                let future_budget =
                    (per_target / 3).min(config::setting("TARGET_FUTURE_MS", 10_000_u64));
                let today_budget = if future_days > 0 {
                    per_target.saturating_sub(future_budget)
                } else {
                    per_target
                };
                let mut winner = None;
                for targets in options {
                    if self.expired() || Self::elapsed(started) >= self.config.budget_ms {
                        break;
                    }
                    let mut child = Planner::new(self.map.problem.clone(), self.base_seed);
                    child.config = self.config.clone();
                    child.config.pool = false;
                    child.config.budget_ms = today_budget;
                    child.config.iterations = 0;
                    child.config.beam_ms = Some((today_budget / 5).min(2000));
                    child.hard_deadline = self.hard_deadline;
                    child.brands_ever = self.brands_ever;
                    child.road_history = self.road_history.clone();
                    child.warm_start = self.warm_start.clone();
                    let plan = child.plan_day_local_target(observation, steps, Some(&targets));
                    let Some(actual) = child.replay(&plan, &child.apos, &child.afuel) else {
                        continue;
                    };
                    let today = actual.claimed.count_ones();
                    let mut daily = today;
                    let mut balls = actual.balls;
                    let mut fuel = actual.fuel.clone();
                    let mut history = self.road_history.clone();
                    history.push(actual.stay.clone());
                    let mut agents: Vec<_> = actual
                        .pos
                        .iter()
                        .enumerate()
                        .map(|(a, &pos)| Agent {
                            kind: self.kinds[a],
                            pos,
                            fuel: actual.fuel[a] as u32,
                        })
                        .collect();
                    let mut warm = None;
                    for offset in 1..=future_days {
                        if self.expired() {
                            break;
                        }
                        let day = observation.day + offset;
                        let next = self.horizon_observation(day, agents.clone(), &history);
                        child.config.budget_ms = (future_budget / future_days as u64).max(1);
                        child.config.beam_ms = Some((child.config.budget_ms / 5).min(1000));
                        child.road_history = history.clone();
                        let future =
                            child.plan_day_local(&next, self.map.problem.day_steps[day] as i32);
                        let Some(result) = child.replay(&future, &child.apos, &child.afuel) else {
                            break;
                        };
                        daily += result.claimed.count_ones();
                        balls += result.balls;
                        fuel = result.fuel.clone();
                        if offset == 1 {
                            warm = Some(WarmStart {
                                observation: next,
                                tours: child.last_tours.clone(),
                                targets: child
                                    .supplies
                                    .iter()
                                    .map(|&s| child.supply_target[s])
                                    .collect(),
                                order: child.order.clone(),
                            });
                        }
                        history.push(result.stay);
                        agents = result
                            .pos
                            .iter()
                            .enumerate()
                            .map(|(a, &pos)| Agent {
                                kind: self.kinds[a],
                                pos,
                                fuel: result.fuel[a] as u32,
                            })
                            .collect();
                    }
                    let reserve = self.patrols.iter().map(|&p| fuel[p]).min().unwrap_or(0) * 10
                        + self.patrols.iter().map(|&p| fuel[p]).sum::<i32>();
                    let key = (
                        (actual.claimed & !self.brands_ever).count_ones(),
                        today,
                        daily,
                        if future_days > 0 { reserve } else { 0 },
                        balls,
                    );
                    eprintln!(
                        "[target search] day={} targets={targets:?} today={today} forecast={daily} reserve={reserve}",
                        observation.day
                    );
                    if winner.as_ref().is_none_or(|(best, _, _, _)| key > *best) {
                        winner = Some((key, plan, actual.claimed, warm));
                    }
                }
                if let Some((_, plan, claimed, warm)) = winner {
                    self.brands_ever |= claimed;
                    self.warm_start = warm;
                    plan
                } else {
                    self.plan_day_local(observation, steps)
                }
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use tester::{engine::Game, generator::generate};
            #[test]
            fn compared_supply_targets_return_a_replayable_plan() {
                let problem = generate(0, None).unwrap().problem;
                let mut game = Game::new(problem.clone(), &[0, 0, 1, 0]);
                let mut planner = Planner::new(problem, 3);
                planner.config.budget_ms = 120;
                planner.config.beam = false;
                planner.set_deadline(1500);
                let plan = planner.plan_day_target_search(&game.observation(0), 32);
                let expected = planner
                    .replay(&plan, &planner.apos, &planner.afuel)
                    .unwrap();
                game.play_day(
                    plan.iter()
                        .map(|a| a.iter().map(|&x| i64::from(x)).collect())
                        .collect(),
                )
                .unwrap();
                assert_eq!(
                    game.score.daily_brands,
                    expected.claimed.count_ones() as usize
                );
                assert_eq!(
                    game.agents.iter().map(|a| a.pos).collect::<Vec<_>>(),
                    expected.pos
                );
                assert_eq!(
                    game.agents
                        .iter()
                        .map(|a| a.fuel as i32)
                        .collect::<Vec<_>>(),
                    expected.fuel
                );
            }
        }
    }

    #[cfg(test)]
    // 元ファイル: solver/tests.rs
    mod tests {
        use super::*;
        use tester::{engine::Game, generator::generate, model::MatchFormat};

        fn fixed_planner(problem: Problem) -> Planner {
            let mut p = Planner::new(problem, 0);
            p.config.beam = false;
            p.config.pool = true;
            p.config.budget_ms = 55_000;
            p.config.iterations = 500;
            p.config.assign_iters = 50;
            p.config.depth = 2;
            p.config.debug = 0;
            p
        }

        #[test]
        #[ignore = "保存済み全日計画をテスタのエンジンで独立検証する実験用"]
        fn verify_saved_solution() {
            let input = std::env::var("PROCON37_VERIFY_INPUT").unwrap();
            let record: serde_json::Value =
                serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
            let problem: Problem = serde_json::from_value(record["problem"].clone()).unwrap();
            let kinds: Vec<u8> = serde_json::from_value(record["kinds"].clone()).unwrap();
            let plans: Vec<tester::model::Plan> =
                serde_json::from_value(record["plans"].clone()).unwrap();
            assert_eq!(plans.len(), problem.day_steps.len());
            let diagnostic = std::env::var_os("PROCON37_VERIFY_IGNORE_FUEL").is_some();
            let mut played_problem = problem.clone();
            if diagnostic {
                played_problem.fuel_limits = 100_000;
            }
            let mut game = Game::new(played_problem, &kinds);
            let mut days = Vec::new();
            for plan in plans {
                let observation = game.observation(0);
                let before = game.score.daily_brands;
                let replay = game.play_day(plan.clone()).unwrap();
                eprintln!(
                    "day={} daily={} agents={:?}",
                    game.day,
                    game.score.daily_brands - before,
                    game.agents
                );
                days.push(
                    serde_json::json!({"plan": plan,"observation":observation,"frames":replay.frames}),
                );
            }
            let out = serde_json::json!({"problem":problem,"kinds":kinds,"days":days,
                "metadata":{"diagnosticIgnoreFuel":diagnostic,"score":game.score}});
            if let Ok(path) = std::env::var("PROCON37_VERIFY_OUTPUT") {
                std::fs::write(path, out.to_string()).unwrap();
            }
            if let Ok(expected) = std::env::var("PROCON37_VERIFY_EXPECT_DAILY") {
                assert!(!diagnostic);
                assert_eq!(game.score.daily_brands, expected.parse::<usize>().unwrap());
            }
        }

        fn replay_checked(p: &mut Planner, game: &mut Game, ops: Plan) {
            let actual = p.replay(&ops, &p.apos, &p.afuel).expect("内部の再生に失敗");
            let before = game.score.clone();
            let replay = game
                .play_day(
                    ops.iter()
                        .map(|a| a.iter().map(|&o| i64::from(o)).collect())
                        .collect(),
                )
                .expect("テスタで不正な回答");
            assert_eq!(
                actual.pos,
                game.agents.iter().map(|a| a.pos).collect::<Vec<_>>()
            );
            assert_eq!(
                actual.fuel,
                game.agents
                    .iter()
                    .map(|a| a.fuel as i32)
                    .collect::<Vec<_>>()
            );
            assert_eq!(actual.balls as u32, game.score.udon - before.udon);
            assert_eq!(
                actual.claimed.count_ones() as usize,
                game.score.daily_brands - before.daily_brands
            );
            let mut stay = vec![0; p.map.cells.len()];
            for frame in &replay.frames[1..] {
                for a in &frame.agents {
                    if p.map.cells[a.pos] == 1 {
                        stay[a.pos] += 1;
                    }
                }
            }
            assert_eq!(actual.stay, stay);
        }

        #[test]
        fn replay_checks_fuel_at_move_completion_after_intermediate_supply() {
            use tester::model::{Map, Spot};
            let problem = Problem {
                starts_at: 0,
                day_seconds: vec![60; 2],
                day_steps: vec![2; 2],
                map: Map {
                    width: 3,
                    height: 1,
                    cells: vec![vec![0; 3]],
                },
                spots: vec![Spot {
                    brand: 0,
                    pos: 2,
                    stocks: 2,
                }],
                agents: vec![0, 1],
                fuel_limits: 3,
                players: 1,
                busy_threshold: 1,
                jammed_threshold: 3,
            };
            problem.validate().unwrap();
            for (supply_kind, supply_pos, accepted) in [(1, 1, true), (1, 2, false), (0, 1, false)]
            {
                let mut game = Game::new(problem.clone(), &[0, supply_kind]);
                // Later-day state: the patrol reserves a two-step move with no fuel.
                game.day = 1;
                game.agents[0].pos = 1;
                game.agents[0].fuel = 0;
                game.agents[1].pos = supply_pos;
                let mut p = fixed_planner(problem.clone());
                p.prepare(&game.observation(0), 2);
                let ops = vec![vec![2], vec![-2]];
                if accepted {
                    replay_checked(&mut p, &mut game, ops);
                    assert_eq!(game.agents[0].fuel, 2);
                    assert_eq!(game.score.udon, 1);
                } else {
                    // Supply only at the destination is too late: fuel is charged
                    // before that step's movement and refill.
                    assert!(p.replay(&ops, &p.apos, &p.afuel).is_none());
                    assert!(game.play_day(vec![vec![2], vec![-2]]).is_err());
                }
            }
        }

        #[test]
        fn representative_matches_agree_with_the_tester() {
            for (seed, rollout_escorts) in [0, 1, 2, 3, 4, 5, 37]
                .into_iter()
                .flat_map(|seed| [(seed, false), (seed, true)])
            {
                let problem = generate(seed, None).unwrap().problem;
                let mut p = fixed_planner(problem.clone());
                p.config.rollout_escorts = rollout_escorts;
                let kinds = p.decide_kinds();
                assert!(kinds.contains(&0));
                let mut game = Game::new(problem, &kinds);
                for day in 0..game.problem.day_steps.len() {
                    let ops = p.plan_day(&game.observation(0), game.problem.day_steps[day] as i32);
                    replay_checked(&mut p, &mut game, ops);
                }
                assert!(game.score.daily_brands > 0);
            }
        }

        #[test]
        fn fixed_iterations_are_reproducible() {
            for seed in [0, 1, 2] {
                let problem = generate(seed, None).unwrap().problem;
                let mut a = fixed_planner(problem.clone());
                let mut b = fixed_planner(problem.clone());
                // Neither stack storage nor the disabled fixed-iteration racing mode
                // may change the chosen fleet or any day's plan.
                a.config.stack_targets = false;
                b.config.assign_race = true;
                b.config.stack_targets = true;
                let kinds = a.decide_kinds();
                assert_eq!(kinds, b.decide_kinds());
                let mut game = Game::new(problem, &kinds);
                for day in 0..game.problem.day_steps.len() {
                    let observation = game.observation(0);
                    let steps = game.problem.day_steps[day] as i32;
                    let ops = a.plan_day(&observation, steps);
                    assert_eq!(ops, b.plan_day(&observation, steps));
                    replay_checked(&mut a, &mut game, ops);
                }
            }
        }

        #[test]
        fn supply_modes_and_experimental_search_emit_legal_plans() {
            // 燃料の少ない条件を用い、同セルの同伴・離れた車両との合流・再給油を通す。
            for (mode, population) in [(0, false), (1, false), (2, false), (3, false), (2, true)] {
                let mut problem = generate(4, Some(MatchFormat::Semifinal)).unwrap().problem;
                problem.fuel_limits = problem.day_steps[0];
                let mut p = fixed_planner(problem.clone());
                p.config.supply_mode = mode;
                p.config.rollout_escorts = true;
                p.config.population = population;
                p.config.iterations = 200;
                p.config.neighbor_mode = usize::from(population);
                p.kinds = vec![0; problem.agents.len()];
                *p.kinds.last_mut().unwrap() = 1;
                let mut game = Game::new(problem, &p.kinds);
                for day in 0..game.problem.day_steps.len() {
                    let obs = game.observation(0);
                    let steps = game.problem.day_steps[day] as i32;
                    p.prepare(&obs, steps);
                    let targets = p.select_supply_targets();
                    p.configure_supply(&targets);
                    let tours = p.greedy_init(false);
                    let mut sim = p.run_alns(tours, Instant::now());
                    p.fill_supply(&mut sim);
                    // 全待機へのフォールバックが問題を隠さないよう、返却直前の計画を検証する。
                    replay_checked(&mut p, &mut game, sim.ops);
                    p.brands_ever |= sim.claimed;
                }
            }
        }

        #[test]
        fn deadline_interrupts_search_without_invalidating_the_plan() {
            let problem = generate(2, None).unwrap().problem;
            let mut p = fixed_planner(problem.clone());
            p.config.iterations = 0;
            p.config.beam = true;
            p.config.beam_ms = Some(55_000);
            p.kinds = vec![0; problem.agents.len()];
            let mut game = Game::new(problem, &p.kinds);
            p.hard_deadline = Some(Instant::now());
            let ops = p.plan_day(&game.observation(0), game.problem.day_steps[0] as i32);
            replay_checked(&mut p, &mut game, ops);
        }

        #[test]
        fn all_beam_granularities_produce_replayable_tours() {
            let problem = generate(0, None).unwrap().problem;
            for mode in 0..=5 {
                let mut p = fixed_planner(problem.clone());
                p.config.beam = true;
                p.config.beam_step = mode;
                p.config.beam_width = 4;
                p.config.beam_width_cap = 4;
                p.config.beam_children_cap = 2048;
                p.config.iterations = 20;
                p.kinds = vec![0; problem.agents.len()];
                let mut game = Game::new(problem.clone(), &p.kinds);
                let ops = p.plan_day(&game.observation(0), game.problem.day_steps[0] as i32);
                replay_checked(&mut p, &mut game, ops);
            }
        }

        #[test]
        fn mulberry32_sequence_and_seed_zero_match_cpp() {
            let mut rng = Prng::new(0);
            let values: Vec<u32> = (0..4).map(|_| (rng.next() * 4294967296.0) as u32).collect();
            assert_eq!(values, vec![1144304738, 1416247, 958946056, 627933444]);
        }
    }

    use config::{Config, Prng};
    use paths::{Dij, INF, MapInfo};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    use tester::model::{Observation, Problem};

    type Tours = Vec<Vec<usize>>;
    type Plan = Vec<Vec<i32>>;

    #[derive(Clone, Default, Debug)]
    struct Sim {
        ops: Plan,
        pbs: Vec<Vec<usize>>,
        claimed: u32,
        balls: i32,
        newkinds: u32,
        future: u32,
        future_newkinds: u32,
        future_balls: i32,
        endpos: Vec<usize>,
        endfuel: Vec<i32>,
        rdv_resume: Vec<Option<i32>>,
        rdv_prefix: Vec<usize>,
        sup_pos: Vec<usize>,
        sup_time: Vec<i32>,
        sup_events: Vec<Vec<(usize, i32, usize)>>,
        sup_overnight: Vec<Option<usize>>,
        sup_mirror: Vec<Option<usize>>,
        sup_detach: Vec<Option<i32>>,
        patrol_escort_slot: Vec<Option<usize>>,
    }

    #[derive(Clone)]
    struct WarmStart {
        observation: Observation,
        tours: Tours,
        targets: Vec<Option<usize>>,
        order: Vec<usize>,
    }

    pub struct Planner {
        map: Arc<MapInfo>,
        pub config: Config,
        rng: Prng,
        base_seed: u32,
        pub kinds: Vec<u8>,
        pub(super) brands_ever: u32,
        na: usize,
        day_steps: i32,
        day_index: usize,
        apos: Vec<usize>,
        afuel: Vec<i32>,
        patrols: Vec<usize>,
        supplies: Vec<usize>,
        order: Vec<usize>,
        supply_target: Vec<Option<usize>>,
        optimistic: Vec<bool>,
        rdv_supply: Vec<Option<usize>>,
        road: Vec<u8>,
        step_cost: Vec<i32>,
        fuel_cost: Vec<i32>,
        precomputed: Vec<bool>,
        distances: Vec<Option<Arc<Dij>>>,
        assignment_distances: Vec<Option<Arc<Dij>>>,
        hard_deadline: Option<Instant>,
        zobrist: beam::Zobrist,
        last_tours: Tours,
        horizon: Option<horizon::Schedule>,
        road_history: Vec<Vec<i32>>,
        path_fuel: i32,
        path_salt: u32,
        warm_start: Option<WarmStart>,
        cover_schedule: Option<coverbeam::CoverSchedule>,
    }

    impl Planner {
        pub fn day_steps(&self, day: usize) -> Option<i32> {
            self.map.problem.day_steps.get(day).map(|&s| s as i32)
        }
        pub fn new(problem: Problem, seed: u32) -> Self {
            let map = Arc::new(MapInfo::new(problem));
            let n = map.cells.len();
            let mut config = Config::from_env();
            if config.atb && map.brands < 20 {
                config.future_balls = true;
            }
            let rng = Prng::new(seed);
            Self {
                map,
                config,
                rng,
                base_seed: rng.state,
                kinds: vec![],
                brands_ever: 0,
                na: 0,
                day_steps: 0,
                day_index: 0,
                apos: vec![],
                afuel: vec![],
                patrols: vec![],
                supplies: vec![],
                order: vec![],
                supply_target: vec![],
                optimistic: vec![],
                rdv_supply: vec![],
                road: vec![0; n],
                step_cost: vec![0; n],
                fuel_cost: vec![0; n],
                precomputed: vec![false; n],
                distances: vec![None; n],
                assignment_distances: vec![None; n],
                hard_deadline: None,
                zobrist: beam::Zobrist::new(n),
                last_tours: vec![],
                horizon: None,
                road_history: vec![],
                path_fuel: config::setting("PATH_FUEL", 0_i32).max(0),
                path_salt: 0,
                warm_start: None,
                cover_schedule: None,
            }
        }
        pub fn set_deadline(&mut self, time_limit_ms: u64) {
            self.hard_deadline = Some(
                Instant::now() + Duration::from_millis(time_limit_ms.saturating_sub(500).max(1)),
            );
        }
        fn expired(&self) -> bool {
            self.hard_deadline.is_some_and(|d| Instant::now() >= d)
        }
        fn elapsed(start: Instant) -> u64 {
            start.elapsed().as_millis() as u64
        }
        fn dij(&mut self, pos: usize) -> Arc<Dij> {
            self.distances[pos]
                .get_or_insert_with(|| {
                    Arc::new(Dij::variant(
                        pos,
                        &self.map,
                        &self.road,
                        self.path_fuel,
                        self.path_salt,
                    ))
                })
                .clone()
        }
        fn distance(&self, from: usize, to: usize) -> i32 {
            self.distances[from].as_ref().map_or(INF, |d| d.dist[to])
        }
        fn prepare(&mut self, observation: &Observation, steps: i32) {
            self.na = observation.agents.len();
            self.kinds = observation.agents.iter().map(|a| a.kind).collect();
            self.day_steps = steps;
            self.day_index = observation.day;
            self.rng = Prng::new(
                0x9e3779b9_u32
                    .wrapping_add((self.day_index as u32).wrapping_mul(0x85ebca6b))
                    .wrapping_add(self.base_seed.wrapping_mul(0xc2b2ae35)),
            );
            if self.rng.state == 0 {
                self.rng.state = 1;
            }
            self.apos = observation.agents.iter().map(|a| a.pos).collect();
            self.afuel = observation.agents.iter().map(|a| a.fuel as i32).collect();
            let previous_road = self.road.clone();
            self.road.fill(0);
            for t in &observation.traffics {
                self.road[t.pos] = t.status;
            }
            if previous_road != self.road {
                self.distances.fill(None);
            }
            self.precomputed.fill(false);
            self.patrols.clear();
            self.supplies.clear();
            for (i, a) in observation.agents.iter().enumerate() {
                if a.kind == 0 {
                    self.patrols.push(i);
                } else {
                    self.supplies.push(i);
                }
            }
            let starts: Vec<_> = self
                .map
                .problem
                .spots
                .iter()
                .map(|s| s.pos)
                .chain(self.patrols.iter().map(|&p| self.apos[p]))
                .collect();
            for pos in starts {
                self.dij(pos);
                self.precomputed[pos] = true;
            }
            for pos in 0..self.map.cells.len() {
                self.step_cost[pos] = self.map.step_cost(pos, &self.road);
                self.fuel_cost[pos] = self.map.fuel_cost(pos);
            }
        }
        fn better(&self, a: &Sim, b: &Sim) -> bool {
            self.key(a) > self.key(b)
        }
        fn key(&self, s: &Sim) -> (u32, u32, u32, u32, i32, i32) {
            (
                s.newkinds,
                if self.config.future_nk {
                    s.future_newkinds
                } else {
                    0
                },
                s.claimed.count_ones(),
                s.future,
                if self.config.future_balls {
                    s.balls + s.future_balls
                } else {
                    0
                },
                s.balls,
            )
        }
        fn collect(
            &self,
            pos: usize,
            stock: &mut [i32],
            visited: &mut u32,
            claimed: &mut u32,
        ) -> bool {
            let Some(si) = self.map.spot_at[pos] else {
                return false;
            };
            let bit = 1_u32 << si;
            if *visited & bit != 0 {
                return false;
            }
            *visited |= bit;
            if stock[si] <= 0 {
                return false;
            }
            stock[si] -= 1;
            *claimed |= 1 << self.map.problem.spots[si].brand;
            true
        }
        fn simulate(&mut self, tours: &Tours) -> Sim {
            let ns = self.supplies.len();
            let mut s = Sim {
                ops: vec![vec![]; self.na],
                pbs: vec![vec![]; self.na],
                endpos: vec![0; self.na],
                endfuel: vec![0; self.na],
                rdv_resume: vec![None; self.na],
                rdv_prefix: vec![0; self.na],
                sup_pos: self.supplies.iter().map(|&p| self.apos[p]).collect(),
                sup_time: vec![0; ns],
                sup_events: vec![vec![]; ns],
                sup_overnight: vec![None; ns],
                sup_mirror: vec![None; ns],
                sup_detach: vec![None; ns],
                patrol_escort_slot: vec![None; self.na],
                ..Sim::default()
            };
            let mut stock: Vec<_> = self
                .map
                .problem
                .spots
                .iter()
                .map(|s| s.stocks as i32)
                .collect();
            if self.config.supply_mode >= 1 {
                for (k, &sup) in self.supplies.iter().enumerate() {
                    if let Some(p) = self.supply_target[sup]
                        && self.optimistic[p]
                        && self.apos[sup] == self.apos[p]
                    {
                        s.sup_time[k] = INF;
                        if self.config.supply_mode == 2 && s.patrol_escort_slot[p].is_none() {
                            s.sup_mirror[k] = Some(p);
                            s.patrol_escort_slot[p] = Some(k);
                        }
                    }
                }
            }
            for p in self.order.clone() {
                self.follow_tour(p, &tours[p], &mut stock, &mut s);
            }
            if self.config.supply_mode >= 1 {
                let mut taken = vec![false; self.na];
                for k in 0..ns {
                    if s.sup_time[k] >= INF {
                        continue;
                    }
                    if self.config.supply_mode == 3 && s.sup_mirror[k].is_some() {
                        let p = s.sup_mirror[k].unwrap();
                        s.sup_overnight[k] = Some(p);
                        taken[p] = true;
                        continue;
                    }
                    let d = self.dij(s.sup_pos[k]);
                    let mut best = None;
                    let mut best_fuel = INF;
                    for &p in &self.patrols {
                        if taken[p]
                            || s.endfuel[p] >= INF / 4
                            || s.sup_time[k] + d.dist[s.endpos[p]] > self.day_steps
                        {
                            continue;
                        }
                        if s.endfuel[p] < best_fuel {
                            best_fuel = s.endfuel[p];
                            best = Some(p);
                        }
                    }
                    if let Some(p) = best {
                        taken[p] = true;
                        s.endfuel[p] = self.map.problem.fuel_limits as i32;
                        s.sup_overnight[k] = Some(p);
                    }
                }
            }
            for &sup in &self.supplies {
                if let Some(p) = self.supply_target[sup]
                    && self.apos[sup] == self.apos[p]
                {
                    s.ops[sup] = s.ops[p].clone();
                }
            }
            s.newkinds = (s.claimed & !self.brands_ever).count_ones();
            s
        }
        fn rendezvous(&self, p: usize, cell: usize, used: i32) -> Option<i32> {
            let sup = self.rdv_supply[p]?;
            let arrive = self.distance(self.apos[sup], cell);
            let resume = used.max(arrive);
            (resume <= self.day_steps).then_some(resume)
        }
        fn follow_tour(&mut self, p: usize, tour: &[usize], stock: &mut [i32], s: &mut Sim) {
            let (mut pos, mut fuel, mut used) = (self.apos[p], self.afuel[p], 0);
            let mut escort = self.optimistic[p];
            let mut escort_sup = if self.config.supply_mode == 2 && escort {
                s.patrol_escort_slot[p]
            } else {
                None
            };
            let mut escort_start = 0;
            let mut escort_moves: Vec<(i32, i32)> = vec![];
            let mut visited = 0;
            let mut plan = vec![];
            let mut pbs = vec![pos; self.day_steps as usize + 1];
            if self.day_steps > 0 && self.collect(pos, stock, &mut visited, &mut s.claimed) {
                s.balls += 1;
            }
            for &target in tour {
                let tp = self.map.problem.spots[target].pos;
                if tp == pos {
                    continue;
                }
                if !self.precomputed[pos] {
                    break;
                }
                let d = self.dij(pos);
                if d.dist[tp] >= INF {
                    continue;
                }
                let dirs = d.path_view(pos, tp);
                if dirs.is_empty() {
                    continue;
                }
                let mut reached = true;
                for &dir in dirs.iter() {
                    let (cost, fc) = (self.step_cost[pos], self.fuel_cost[pos]);
                    if !escort && fuel < fc && self.config.supply_mode >= 1 {
                        let mut best_k = None;
                        let mut best_resume = INF;
                        for k in 0..self.supplies.len() {
                            if s.sup_time[k] >= INF {
                                continue;
                            }
                            let d = self.dij(s.sup_pos[k]);
                            let resume = used.max(s.sup_time[k] + d.dist[pos]);
                            if resume > self.day_steps || resume >= best_resume {
                                continue;
                            }
                            if self.config.supply_mode == 3
                                && let Some(home) = s.sup_mirror[k]
                            {
                                let back = self.dij(pos);
                                if resume + back.dist[s.endpos[home]] > self.day_steps {
                                    continue;
                                }
                            }
                            best_k = Some(k);
                            best_resume = resume;
                        }
                        if let Some(k) = best_k {
                            pbs[used as usize + 1..=best_resume as usize].fill(pos);
                            if best_resume > used {
                                plan.push(-(best_resume - used));
                                used = best_resume;
                            }
                            if s.rdv_resume[p].is_none() {
                                s.rdv_resume[p] = Some(best_resume);
                                s.rdv_prefix[p] = plan.len();
                            }
                            fuel = self.map.problem.fuel_limits as i32;
                            s.sup_events[k].push((pos, best_resume, p));
                            if self.config.supply_mode >= 2 && s.sup_mirror[k].is_none() {
                                s.sup_mirror[k] = Some(p);
                                s.sup_pos[k] = pos;
                                s.sup_time[k] = INF;
                                escort = true;
                                escort_sup = Some(k);
                                escort_start = best_resume;
                            } else {
                                s.sup_pos[k] = pos;
                                s.sup_time[k] = best_resume;
                            }
                        }
                    } else if !escort
                        && fuel < fc
                        && s.rdv_resume[p].is_none()
                        && let Some(resume) = self.rendezvous(p, pos, used)
                    {
                        pbs[used as usize + 1..=resume as usize].fill(pos);
                        if resume > used {
                            plan.push(-(resume - used));
                            used = resume;
                        }
                        s.rdv_resume[p] = Some(resume);
                        s.rdv_prefix[p] = plan.len();
                        fuel = self.map.problem.fuel_limits as i32;
                        escort = true;
                    }
                    if used + cost > self.day_steps || (!escort && fuel < fc) {
                        reached = false;
                        break;
                    }
                    plan.push(dir);
                    let dest = self.map.neighbors[pos][dir as usize].unwrap();
                    pbs[used as usize + 1..(used + cost) as usize].fill(pos);
                    used += cost;
                    pbs[used as usize] = dest;
                    if !escort {
                        fuel -= fc;
                    }
                    if escort && escort_sup.is_some() {
                        escort_moves.push((used, fc));
                    }
                    pos = dest;
                    if self.collect(pos, stock, &mut visited, &mut s.claimed) {
                        s.balls += 1;
                    }
                }
                if !reached {
                    break;
                }
            }
            pbs[used as usize + 1..].fill(pos);
            s.endpos[p] = pos;
            s.endfuel[p] = if escort { INF / 2 } else { fuel };
            if let Some(k) = escort_sup {
                let cap = self.map.problem.fuel_limits as i32;
                let mut suffix = 0;
                let mut cut = escort_moves.len();
                for i in (0..cut).rev() {
                    if suffix + escort_moves[i].1 > cap {
                        break;
                    }
                    suffix += escort_moves[i].1;
                    cut = i;
                }
                if cut == 0
                    && escort_start == 0
                    && !escort_moves.is_empty()
                    && suffix > self.afuel[p]
                {
                    cut = 1;
                    suffix -= escort_moves[0].1;
                }
                let detach = if cut == 0 {
                    escort_start
                } else {
                    escort_moves[cut - 1].0
                };
                s.sup_detach[k] = Some(detach);
                s.sup_pos[k] = pbs[detach as usize];
                s.sup_time[k] = detach;
                if self.config.supply_mode != 3 {
                    s.endfuel[p] = cap - suffix;
                }
            }
            if self.config.supply_mode != 1
                && !escort
                && s.rdv_resume[p].is_none()
                && let Some(sup) = self.rdv_supply[p]
                && self.distance(self.apos[sup], pos) <= self.day_steps
            {
                s.endfuel[p] = self.map.problem.fuel_limits as i32;
            }
            if used < self.day_steps {
                plan.push(-(self.day_steps - used));
            }
            if plan.is_empty() {
                plan.push(-self.day_steps);
            }
            s.ops[p] = plan;
            s.pbs[p] = pbs;
        }

        pub fn plan_day(&mut self, observation: &Observation, steps: i32) -> Plan {
            if self.cover_schedule.is_some()
                && let Some(plan) = self.cover_beam_day(observation)
            {
                return plan;
            }
            if config::setting("HORIZON", 0) != 0 {
                return self.horizon_day(observation, steps);
            }
            let plan = if config::setting("TARGET_SEARCH", 0) != 0
                && observation.agents.iter().filter(|a| a.kind == 1).count() == 1
            {
                self.plan_day_target_search(observation, steps)
            } else {
                self.plan_day_local(observation, steps)
            };
            if let Some(actual) = self.replay(&plan, &self.apos, &self.afuel) {
                if observation.day == 0 {
                    self.road_history.clear();
                }
                self.road_history.push(actual.stay);
            }
            plan
        }

        fn plan_day_local(&mut self, observation: &Observation, steps: i32) -> Plan {
            self.plan_day_local_target(observation, steps, None)
        }

        fn plan_day_local_target(
            &mut self,
            observation: &Observation,
            steps: i32,
            forced_targets: Option<&[Option<usize>]>,
        ) -> Plan {
            self.prepare(observation, steps);
            let targets = forced_targets
                .map(|t| t.to_vec())
                .unwrap_or_else(|| self.select_supply_targets());
            self.configure_supply(&targets);
            let start = Instant::now();
            let mut tours = self.greedy_init(false);
            if config::setting("EXACT_INIT", 0) != 0
                && self.map.brands <= 16
                && let Some(exact) = self.exact_partition()
            {
                tours = exact;
            }
            if let Some(warm) = self.warm_start.take()
                && warm.observation.day == observation.day
                && warm.observation.agents == observation.agents
                && warm.observation.traffics == observation.traffics
                && (forced_targets.is_none() || warm.targets == targets)
            {
                self.configure_supply(&warm.targets);
                self.order = warm.order;
                tours = warm.tours;
            }
            let mut beam_ms = self.config.beam_ms.unwrap_or_else(|| {
                if self.config.budget_ms >= 20_000 {
                    5000
                } else {
                    200.max(self.config.budget_ms / 10)
                }
            });
            if matches!(self.config.beam_step, 3..=5) && self.config.beam_ms.is_none() {
                beam_ms = beam_ms.max(self.config.budget_ms.saturating_sub(3000));
            }
            if self.config.iterations > 0 {
                beam_ms = beam_ms.min(500);
            }
            if self.config.beam {
                self.beam_init(start, beam_ms, &mut tours);
            } else if self.config.init_mode == 1 {
                let mut best = self.simulate(&tours);
                self.rollout(&mut best);
                while Self::elapsed(start) < beam_ms && !self.expired() {
                    let cand = self.greedy_init(true);
                    let mut s = self.simulate(&cand);
                    self.rollout(&mut s);
                    if self.better(&s, &best) {
                        tours = cand;
                        best = s;
                    }
                }
            }
            let mut best = self.run_alns(tours, start);
            self.fill_supply(&mut best);
            if config::setting("REPAIR_DAY", 0) != 0
                && let Some(plan) =
                    self.repair_full_day(&best.ops, config::setting("REPAIR_MS", 2000_u64))
            {
                best.ops = plan;
            }
            let actual = self.replay(&best.ops, &self.apos, &self.afuel);
            if let Some(actual) = actual {
                self.brands_ever |= if self.config.exact_claimed {
                    actual.claimed
                } else {
                    best.claimed
                };
                best.ops
            } else {
                eprintln!(
                    "warning: invalid plan; falling back to all-wait (day={})",
                    self.day_index
                );
                let plan = vec![vec![-steps.max(1)]; self.na];
                if let Some(actual) = self.replay(&plan, &self.apos, &self.afuel) {
                    self.brands_ever |= actual.claimed;
                }
                plan
            }
        }
    }
}

use solver::Planner;
use std::{
    io::{self, BufRead, Write},
    time::{SystemTime, UNIX_EPOCH},
};
use tester::model::{Observation, PROTOCOL_VERSION, Problem, Request, Response};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let seed = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("PROCON37_SEED").ok())
        .map(|s| s.parse::<u64>().map(|n| n as u32))
        .transpose()?
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u32
        });
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let mut planner: Option<Planner> = None;
    let mut before_previous = 0;
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&line)?;
        let response = if value.get("protocolVersion").is_some() {
            let request: Request = serde_json::from_value(value)?;
            if request.protocol_version != PROTOCOL_VERSION {
                return Err("unsupported protocol version".into());
            }
            let answer = match request.phase.as_str() {
                "select" => {
                    let mut p = Planner::new(request.problem.ok_or("missing problem")?, seed);
                    p.set_deadline(request.time_limit_ms);
                    let answer = serde_json::to_value(p.decide_kinds())?;
                    planner = Some(p);
                    before_previous = 0;
                    answer
                }
                "plan" => {
                    let p = planner.as_mut().ok_or("plan received before select")?;
                    if request.previous_accepted == Some(false) {
                        p.brands_ever = before_previous;
                    }
                    before_previous = p.brands_ever;
                    p.set_deadline(request.time_limit_ms);
                    serde_json::to_value(p.plan_day(
                        &request.state.ok_or("missing state")?,
                        request.steps.ok_or("missing steps")? as i32,
                    ))?
                }
                _ => return Err("unknown phase".into()),
            };
            serde_json::to_value(Response {
                request_id: request.request_id,
                answer,
            })?
        } else if let Some(p) = planner.as_mut() {
            let state: Observation = serde_json::from_value(value)?;
            let steps = p.day_steps(state.day).ok_or("day out of range")?;
            p.set_deadline(p.config.budget_ms.saturating_add(4500));
            serde_json::to_value(p.plan_day(&state, steps))?
        } else {
            let problem: Problem = serde_json::from_value(value)?;
            let mut p = Planner::new(problem, seed);
            p.set_deadline(p.config.assign_ms.saturating_add(4500));
            let answer = serde_json::to_value(p.decide_kinds())?;
            planner = Some(p);
            answer
        };
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}
