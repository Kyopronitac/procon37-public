use crate::model::*;
use anyhow::{Context, Result};
use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};
use std::{cmp::Reverse, collections::BinaryHeap};

pub const GENERATOR_VERSION: &str = "hex-udon-v24";
const RESOURCE_LIMITED_PERCENT: usize = 80;
const SPREAD_LAYOUT_PERCENT: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GenerationProfile {
    Broad,
    ResourceLimited,
}

fn generation_profile(seed: u64, format: MatchFormat) -> GenerationProfile {
    if Random::new(seed, 5, Some(format)).below(100) < RESOURCE_LIMITED_PERCENT {
        GenerationProfile::ResourceLimited
    } else {
        GenerationProfile::Broad
    }
}
const SAMPLES: [&str; 3] = [
    include_str!("../../official/sample_maps/sample_16.json"),
    include_str!("../../official/sample_maps/sample_24.json"),
    include_str!("../../official/sample_maps/sample_32.json"),
];
// 既存ケースを先頭に残し、追加分をサイズ別に固定シャッフルした盤面群。
const ARENA_CASES: [[&str; 5]; 3] = [
    [
        include_str!("../fixtures/arena/match-937.json"),
        include_str!("../fixtures/arena/pasted-16-01.json"),
        include_str!("../fixtures/arena/pasted-16-04.json"),
        include_str!("../fixtures/arena/pasted-16-02.json"),
        include_str!("../fixtures/arena/pasted-16-03.json"),
    ],
    [
        include_str!("../fixtures/arena/match-941.json"),
        include_str!("../fixtures/arena/pasted-24-03.json"),
        include_str!("../fixtures/arena/pasted-24-02.json"),
        include_str!("../fixtures/arena/pasted-24-01.json"),
        include_str!("../fixtures/arena/pasted-24-04.json"),
    ],
    [
        include_str!("../fixtures/arena/match-916.json"),
        include_str!("../fixtures/arena/pasted-32-04.json"),
        include_str!("../fixtures/arena/pasted-32-03.json"),
        include_str!("../fixtures/arena/pasted-32-02.json"),
        include_str!("../fixtures/arena/pasted-32-01.json"),
    ],
];
fn arena_case(seed: u64, format: MatchFormat) -> Option<&'static str> {
    if seed % 4 != 3 {
        return None;
    }
    let bank = &ARENA_CASES[match format {
        MatchFormat::First => 0,
        MatchFormat::Semifinal => 1,
        MatchFormat::Final => 2,
    }];
    // 自動形式の各サイズは12 seedごとに現れる。同じseedの形式明示でも同じ盤面。
    let index = (seed / 12 % bank.len() as u64) as usize;
    Some(bank[index])
}
struct Random(ChaCha8Rng);
impl Random {
    fn new(seed: u64, domain: u64, format: Option<MatchFormat>) -> Self {
        let mut bytes = [0; 32];
        for (i, n) in [
            seed,
            domain,
            format.map_or(0, |f| f.parameters().0 as u64),
            1,
        ]
        .iter()
        .enumerate()
        {
            bytes[i * 8..i * 8 + 8].copy_from_slice(&n.to_le_bytes());
        }
        Self(ChaCha8Rng::from_seed(bytes))
    }
    fn below(&mut self, n: usize) -> usize {
        assert!(n > 0);
        let n = n as u64;
        let threshold = n.wrapping_neg() % n;
        loop {
            let v = self.0.next_u64();
            if v >= threshold {
                return (v % n) as usize;
            }
        }
    }
    fn range(&mut self, low: usize, high: usize) -> usize {
        low + self.below(high - low + 1)
    }
    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

pub fn resolve_format(seed: u64, format: Option<MatchFormat>) -> MatchFormat {
    format.unwrap_or_else(|| {
        [
            MatchFormat::First,
            MatchFormat::Semifinal,
            MatchFormat::Final,
        ][(seed % 3) as usize]
    })
}

fn fixed_problem(seed: u64, format: MatchFormat, arena_maps: bool) -> Result<Option<Problem>> {
    if seed < SAMPLES.len() as u64 && format == resolve_format(seed, None) {
        Ok(Some(
            serde_json::from_str(SAMPLES[seed as usize]).context("公式サンプルを読み込めません")?,
        ))
    } else if arena_maps && let Some(json) = arena_case(seed, format) {
        let mut problem: Problem =
            serde_json::from_str(json).context("練習場の回帰ケースを読み込めません")?;
        problem.starts_at = 0;
        Ok(Some(problem))
    } else {
        Ok(None)
    }
}

fn generated_schedule(seed: u64, format: MatchFormat) -> (Random, Vec<u32>) {
    let mut params = Random::new(seed, 1, Some(format));
    let days = params.range(4, 10);
    (params, vec![60; days])
}

/// Read the same schedule as `generate` without building the map. Values are seconds.
pub(crate) fn day_seconds(
    seed: u64,
    format: Option<MatchFormat>,
    arena_maps: bool,
) -> Result<Vec<u32>> {
    let format = resolve_format(seed, format);
    Ok(
        if let Some(problem) = fixed_problem(seed, format, arena_maps)? {
            problem.day_seconds
        } else {
            generated_schedule(seed, format).1
        },
    )
}

pub fn generate(seed: u64, format: Option<MatchFormat>) -> Result<Case> {
    generate_with_arena_maps(seed, format, true)
}

/// Disabling arena maps keeps the official samples and uses procedural maps for all other seeds.
pub fn generate_with_arena_maps(
    seed: u64,
    format: Option<MatchFormat>,
    arena_maps: bool,
) -> Result<Case> {
    let format = resolve_format(seed, format);
    if let Some(mut problem) = fixed_problem(seed, format, arena_maps)? {
        problem.players = 1;
        problem.validate()?;
        return Ok(Case {
            generator_version: GENERATOR_VERSION.into(),
            seed: seed.to_string(),
            format,
            problem,
        });
    }
    let (w, a, kmin, kmax, _) = format.parameters();
    let (mut params, day_seconds) = generated_schedule(seed, format);
    let days = day_seconds.len();
    let mut trend = Random::new(seed, 4, Some(format));
    let profile = generation_profile(seed, format);
    let day_steps = make_day_steps(w, days, &mut params, &mut trend, profile);
    let initial = day_steps[0] as usize;
    let max_fuel = match profile {
        GenerationProfile::Broad => 3 * initial,
        GenerationProfile::ResourceLimited => 5 * initial / 4,
    };
    let fuel_limits = params.range(initial, max_fuel) as u32;
    let pairs = (1..=5)
        .flat_map(|b| (2..=10).filter(move |&j| j > b).map(move |j| (b, j)))
        .filter(|&(b, j)| profile == GenerationProfile::Broad || (b <= 2 && j <= 4))
        .collect::<Vec<_>>();
    let (busy_threshold, jammed_threshold) = pairs[params.below(pairs.len())];
    let brands = params.range(kmin, kmax);
    let mut terrain = Random::new(seed, 2, Some(format));
    let mut layout = Random::new(seed, 3, Some(format));
    let spread = Random::new(seed, 6, Some(format)).below(100) < SPREAD_LAYOUT_PERCENT;
    let mut spacing = Random::new(seed, 7, Some(format));
    let (map, positions) = make_map(w, a, &mut terrain, &mut layout, spread, &mut spacing)?;
    let mut assigned = (0..brands)
        .chain((brands..w).map(|_| layout.below(brands)))
        .collect::<Vec<_>>();
    layout.shuffle(&mut assigned);
    let mut spots = (0..w)
        .map(|i| Spot {
            brand: assigned[i],
            pos: positions[i],
            stocks: layout.range(1, a) as u32,
        })
        .collect::<Vec<_>>();
    spots.sort_by_key(|s| s.pos);
    let mut ids = vec![None; brands];
    let mut next = 0;
    for s in &mut spots {
        s.brand = *ids[s.brand].get_or_insert_with(|| {
            let n = next;
            next += 1;
            n
        });
    }
    spots.sort_by_key(|s| (s.brand, s.pos));
    let mut agents = positions[w..w + a].to_vec();
    agents.sort_unstable();
    let problem = Problem {
        starts_at: 0,
        day_seconds,
        day_steps,
        map,
        spots,
        agents,
        fuel_limits,
        players: 1,
        busy_threshold,
        jammed_threshold,
    };
    problem.validate()?;
    Ok(Case {
        generator_version: GENERATOR_VERSION.into(),
        seed: seed.to_string(),
        format,
        problem,
    })
}

fn make_day_steps(
    w: usize,
    days: usize,
    params: &mut Random,
    trend: &mut Random,
    profile: GenerationProfile,
) -> Vec<u32> {
    // Most schedules start at the sample minimum. Retain larger initial values
    // in the broad profile; neither profile restricts the growth scale.
    let initial_max = match profile {
        GenerationProfile::Broad => 8 * w,
        GenerationProfile::ResourceLimited => 2 * w,
    };
    let mut current = params.range(2 * w, initial_max);
    let mut steps = vec![current as u32];
    // The expected increment is near W/4: about 50% growth over five days, with
    // independent variation between matches and between individual days.
    let growth = trend.range(w / 16, 7 * w / 16);
    for day in 1..days {
        let room = 8 * w - current;
        let remaining = days - day;
        let limit = (2 * growth).min(2 * room / remaining).min(room);
        current += params.range(0, limit);
        steps.push(current as u32);
    }
    steps
}

fn hex_distance(a: usize, b: usize, w: usize) -> usize {
    let (ay, by) = ((a / w) as isize, (b / w) as isize);
    let dq = (a % w) as isize - (ay + 1) / 2 - (b % w) as isize + (by + 1) / 2;
    let dr = ay - by;
    (dq.unsigned_abs() + dr.unsigned_abs() + (dq + dr).unsigned_abs()) / 2
}

fn spread_spots(w: usize, rng: &mut Random) -> Vec<usize> {
    let edge_count = rng.range(w / 4, w / 2);
    let candidates = rng.range(1, 3);
    let mut kinds = (0..w)
        .map(|i| usize::from(i < edge_count))
        .collect::<Vec<_>>();
    rng.shuffle(&mut kinds);
    let mut pools = [Vec::new(), Vec::new()];
    for p in 0..w * w {
        let edge = p % w == 0 || p % w == w - 1 || p / w == 0 || p / w == w - 1;
        pools[usize::from(edge)].push(p);
    }
    let mut positions = Vec::with_capacity(w);
    for kind in kinds {
        let pool = &mut pools[kind];
        let count = candidates.min(pool.len());
        for i in 0..count {
            let j = rng.range(i, pool.len() - 1);
            pool.swap(i, j);
        }
        // Prefer space among a few random candidates, without a minimum-distance
        // constraint. One-candidate cases retain ordinary random clustering.
        let best = (0..count)
            .max_by_key(|&i| {
                positions
                    .iter()
                    .map(|&p| hex_distance(pool[i], p, w))
                    .min()
                    .unwrap_or(0)
            })
            .unwrap();
        positions.push(pool.swap_remove(best));
    }
    positions
}

fn make_map(
    w: usize,
    agents: usize,
    terrain: &mut Random,
    layout: &mut Random,
    spread: bool,
    spacing: &mut Random,
) -> Result<(Map, Vec<usize>)> {
    // Retry an entire layout rather than violating the road-width or spot constraints.
    for _ in 0..128 {
        let mut map = chunk_map(w, terrain);
        let mut positions = if spread {
            spread_spots(w, spacing)
        } else {
            let mut positions = (0..map.len()).collect::<Vec<_>>();
            layout.shuffle(&mut positions);
            positions.truncate(w);
            positions
        };
        let mut reserved = vec![false; map.len()];
        for &p in &positions {
            reserved[p] = true;
            map.set(p, 0);
        }
        let mut origins = positions.clone();
        layout.shuffle(&mut origins);
        for &spot in &origins {
            if map.neighbors(spot).any(|p| map.terrain(p) == 1) {
                continue;
            }
            let starts = map
                .neighbors(spot)
                .filter(|&p| !reserved[p] && thin_addition(&map, p))
                .collect::<Vec<_>>();
            if starts.is_empty() {
                break;
            }
            let start = starts[layout.below(starts.len())];
            grow_road(&mut map, &reserved, start, layout);
        }
        if positions
            .iter()
            .any(|&s| !map.neighbors(s).any(|p| map.terrain(p) == 1))
        {
            continue;
        }
        // Sample road areas span roughly 24–30%. Extend away from spots so
        // reaching the area target does not add entrances around every spot.
        let target = map.len() * layout.range(23, 31) / 100;
        while (0..map.len()).filter(|&p| map.terrain(p) == 1).count() < target {
            let starts = (0..map.len())
                .filter(|&p| {
                    !reserved[p]
                        && map.terrain(p) != 1
                        && !map.neighbors(p).any(|q| reserved[q])
                        && map.neighbors(p).filter(|&q| map.terrain(q) == 1).count() == 1
                        && thin_addition(&map, p)
                })
                .collect::<Vec<_>>();
            if starts.is_empty() {
                break;
            }
            let start = starts[layout.below(starts.len())];
            grow_road(&mut map, &reserved, start, layout);
        }
        connect(&mut map);
        let counts = [0, 1, 2, 3].map(|t| (0..map.len()).filter(|&p| map.terrain(p) == t).count());
        if counts
            .iter()
            .any(|&n| n * 100 < map.len() * 15 || n * 100 > map.len() * 38)
        {
            continue;
        }
        let mut available = (0..map.len())
            .filter(|&p| map.terrain(p) == 0 && !reserved[p])
            .collect::<Vec<_>>();
        if available.len() < agents {
            continue;
        }
        layout.shuffle(&mut available);
        positions.extend_from_slice(&available[..agents]);
        return Ok((map, positions));
    }
    anyhow::bail!("幅1の道路と地形比率を満たすマップを128回以内に生成できませんでした")
}

fn chunk_map(w: usize, rng: &mut Random) -> Map {
    let mut map = Map {
        width: w,
        height: w,
        cells: vec![vec![255; w]; w],
    };
    let mut order = (0..map.len()).collect::<Vec<_>>();
    rng.shuffle(&mut order);
    let mut counts = [0; 3];
    // Samples vary in terrain proportions. Choose a modest map-wide bias,
    // retaining small growth units and checking the final map proportions.
    let weights = [rng.range(3, 5), rng.range(3, 5), rng.range(3, 5)];
    for start in order {
        if map.terrain(start) != 255 {
            continue;
        }
        // Fill each terrain in proportion to its weight; break ties randomly.
        let mut kinds = [0, 1, 2];
        rng.shuffle(&mut kinds);
        let kind = *kinds
            .iter()
            .min_by(|&&a, &&b| (counts[a] * weights[b]).cmp(&(counts[b] * weights[a])))
            .unwrap();
        // Small growth units merge naturally. Larger units made final components
        // several times larger than those in the official samples.
        let target = rng.range(1, 2);
        let mut frontier = vec![start];
        for _ in 0..target {
            if frontier.is_empty() {
                break;
            }
            let i = rng.below(frontier.len());
            let p = frontier.swap_remove(i);
            map.set(p, [0, 2, 3][kind]);
            counts[kind] += 1;
            for q in map.neighbors(p).collect::<Vec<_>>() {
                if map.terrain(q) == 255 && !frontier.contains(&q) {
                    frontier.push(q);
                }
            }
        }
    }
    map
}

// Forbid two filled triangles sharing an edge (the smallest two-cell-wide patch).
// A single triangle at a bend/junction is allowed on the hexagonal grid.
fn thin_addition(map: &Map, p: usize) -> bool {
    let road = |q| q == p || map.terrain(q) == 1;
    let mut local = map.neighbors(p).collect::<Vec<_>>();
    local.push(p);
    for &a in &local {
        if !road(a) {
            continue;
        }
        for b in map.neighbors(a).filter(|&b| a < b && road(b)) {
            if map
                .neighbors(a)
                .filter(|&c| road(c) && map.neighbors(b).any(|d| c == d))
                .count()
                > 1
            {
                return false;
            }
        }
    }
    true
}

fn grow_road(map: &mut Map, spots: &[bool], start: usize, rng: &mut Random) {
    let mut pos = start;
    // Leave the originating spot after one or two adjacent road cells, instead
    // of winding around it. Other routes may still meet the same spot.
    let leave_immediately = rng.below(2) == 0;
    let initial_spots = map
        .neighbors(start)
        .filter(|&q| spots[q])
        .collect::<Vec<_>>();
    let mut direction = rng.below(6);
    let limit = rng.range(map.width / 2, 2 * map.width);
    for step in 0..limit {
        // Contact with an older road or another spot completes this route.
        let contacts = map.neighbors(pos).filter(|&q| map.terrain(q) == 1).count();
        map.set(pos, 1);
        if step > 0
            && (contacts > 1
                || map
                    .neighbors(pos)
                    .any(|q| spots[q] && !initial_spots.contains(&q)))
        {
            break;
        }
        let mut choices = Vec::new();
        for d in 0..6 {
            if let Some(q) = map.neighbor(pos, d)
                && !spots[q]
                && map.terrain(q) != 1
                && ((step == 0 && !leave_immediately)
                    || !map.neighbors(q).any(|s| initial_spots.contains(&s)))
                && thin_addition(map, q)
            {
                choices.push((q, d));
                if d == direction {
                    choices.push((q, d));
                }
            }
        }
        if choices.is_empty() {
            break;
        }
        (pos, direction) = choices[rng.below(choices.len())];
    }
}

fn connect(map: &mut Map) {
    let n = map.len();
    let start = (0..n).find(|&p| map.terrain(p) != 3).unwrap();
    loop {
        let seen = map.reachable(start);
        if (0..n).all(|p| map.terrain(p) == 3 || seen[p]) {
            break;
        }
        let mut dist = vec![(usize::MAX, usize::MAX); n];
        let mut parent = vec![usize::MAX; n];
        let mut queue = BinaryHeap::new();
        for (p, &s) in seen.iter().enumerate() {
            if s {
                dist[p] = (0, 0);
                queue.push(Reverse((0, 0, p)));
            }
        }
        let target = loop {
            let Reverse((cost, steps, p)) = queue.pop().unwrap();
            if dist[p] != (cost, steps) {
                continue;
            }
            if !seen[p] && map.terrain(p) != 3 {
                break p;
            }
            for q in map.neighbors(p) {
                let next = (cost + usize::from(map.terrain(q) == 3), steps + 1);
                if next < dist[q] {
                    dist[q] = next;
                    parent[q] = p;
                    queue.push(Reverse((next.0, next.1, q)));
                }
            }
        };
        let mut p = target;
        while !seen[p] {
            if map.terrain(p) == 3 {
                map.set(p, 0);
            }
            p = parent[p];
        }
    }
}

/// Identical maps, schedules, fuel, stock, and starts; only the player count changes.
pub fn generate_for_players(seed: u64, format: Option<MatchFormat>, players: u32) -> Result<Case> {
    generate_for_players_with_arena_maps(seed, format, players, true)
}

pub fn generate_for_players_with_arena_maps(
    seed: u64,
    format: Option<MatchFormat>,
    players: u32,
    arena_maps: bool,
) -> Result<Case> {
    let mut case = generate_with_arena_maps(seed, format, arena_maps)?;
    case.problem.players = players;
    case.problem.validate_players(players)?;
    Ok(case)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_format_cycles_evenly_and_explicit_format_wins() {
        let formats = [
            MatchFormat::First,
            MatchFormat::Semifinal,
            MatchFormat::Final,
        ];
        let mut counts = [0; 3];
        for seed in 0..60 {
            assert_eq!(resolve_format(seed, None), formats[(seed % 3) as usize]);
            assert_eq!(
                resolve_format(seed, Some(MatchFormat::Final)),
                MatchFormat::Final
            );
            counts[(seed % 3) as usize] += 1;
        }
        assert_eq!(counts, [20, 20, 20]);
        assert_eq!(
            resolve_format(u64::MAX, None),
            formats[(u64::MAX % 3) as usize]
        );
    }
    #[test]
    fn arena_cases_are_inserted_once_per_four_seeds_and_preserve_the_source_problem() {
        for format in [
            None,
            Some(MatchFormat::First),
            Some(MatchFormat::Semifinal),
            Some(MatchFormat::Final),
        ] {
            for block in 0..15 {
                let inserted = (block * 4..block * 4 + 4)
                    .filter(|&seed| arena_case(seed, resolve_format(seed, format)).is_some())
                    .collect::<Vec<_>>();
                assert_eq!(inserted, vec![block * 4 + 3]);
                let seed = inserted[0];
                let f = resolve_format(seed, format);
                let mut source: Problem =
                    serde_json::from_str(arena_case(seed, f).unwrap()).unwrap();
                source.validate_players(10).unwrap();
                source.starts_at = 0;
                for players in [1, 5, 10, 16] {
                    source.players = players;
                    let case = generate_for_players(seed, format, players).unwrap();
                    assert_eq!(case.problem, source);
                    assert_eq!(day_seconds(seed, format, true).unwrap(), source.day_seconds);
                    assert_eq!(case, generate_for_players(seed, Some(f), players).unwrap());
                }
            }
        }
        for (seed, width, first_steps) in [(3, 16, 122), (7, 24, 146), (11, 32, 93)] {
            let case = generate(seed, None).unwrap();
            assert_eq!(case.problem.map.width, width);
            assert_eq!(case.problem.day_steps[0], first_steps);
        }
    }

    #[test]
    fn disabling_arena_maps_keeps_samples_and_generates_the_replaced_seeds() {
        for format in [
            None,
            Some(MatchFormat::First),
            Some(MatchFormat::Semifinal),
            Some(MatchFormat::Final),
        ] {
            for seed in 0..60 {
                let included = generate_with_arena_maps(seed, format, true).unwrap();
                let excluded = generate_with_arena_maps(seed, format, false).unwrap();
                assert_eq!(included, generate(seed, format).unwrap());
                assert_eq!(
                    excluded,
                    generate_with_arena_maps(seed, Some(excluded.format), false).unwrap()
                );
                assert_eq!(
                    excluded.problem.day_seconds,
                    day_seconds(seed, format, false).unwrap()
                );
                if seed % 4 == 3 {
                    assert_ne!(included.problem.map, excluded.problem.map);
                    assert!(
                        excluded
                            .problem
                            .day_seconds
                            .iter()
                            .all(|&seconds| seconds == 60)
                    );
                } else {
                    assert_eq!(included, excluded);
                }
                for players in [5, 16] {
                    let multiplayer =
                        generate_for_players_with_arena_maps(seed, format, players, false).unwrap();
                    let mut expected = excluded.problem.clone();
                    expected.players = players;
                    assert_eq!(multiplayer.problem, expected);
                }
            }
        }
    }

    #[test]
    fn arena_case_provenance_separates_public_scores_from_user_pastes() {
        let sources: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/arena/sources.json")).unwrap();
        let cases = sources["cases"].as_array().unwrap();
        assert_eq!(
            cases.len(),
            ARENA_CASES.iter().map(|bank| bank.len()).sum::<usize>()
        );
        let public_cases = cases.iter().filter(|case| case["source"] == "public-api");
        assert_eq!(public_cases.clone().count(), 3);
        for case in public_cases {
            assert_eq!(case["team"], "終焉の使徒");
            assert!(case["score"]["udon"].as_u64().unwrap() > 0);
            assert!(case["score"]["uniqueBrands"].as_u64().unwrap() > 0);
            assert!(case["score"]["dailyBrands"].as_u64().unwrap() > 0);
        }
        let provided = cases.iter().filter(|case| case["source"] == "user-paste");
        assert_eq!(provided.clone().count(), 12);
        for case in provided {
            assert_eq!(case["providedOn"], "2026-10-04");
            assert!(case.get("score").is_none());
            assert!(case.get("rank").is_none());
        }
    }

    #[test]
    fn automatic_first_sixty_seeds_use_every_arena_map_once() {
        let mut maps = Vec::new();
        let mut arena_problems = Vec::new();
        let mut counts = [0; 3];
        for seed in 0..60 {
            let case = generate(seed, None).unwrap();
            assert!(!maps.contains(&case.problem.map), "盤面が重複: seed {seed}");
            maps.push(case.problem.map.clone());
            if seed % 4 == 3 {
                counts[(seed % 3) as usize] += 1;
                arena_problems.push(case.problem);
            }
        }
        assert_eq!(counts, [5, 5, 5]);
        for (format, bank) in [
            MatchFormat::First,
            MatchFormat::Semifinal,
            MatchFormat::Final,
        ]
        .into_iter()
        .zip(ARENA_CASES)
        {
            for json in bank {
                let mut source: Problem = serde_json::from_str(json).unwrap();
                source.validate_players(10).unwrap();
                assert_eq!(source.map.width, format.parameters().0);
                source.starts_at = 0;
                source.players = 1;
                assert_eq!(arena_problems.iter().filter(|p| **p == source).count(), 1);
            }
        }
        // 60 seed周期。u64の上限でもインデックス計算はオーバーフローしない。
        for seed in [3, 7, 11, 15, 27, 39, 51, u64::MAX - 60] {
            for format in [
                MatchFormat::First,
                MatchFormat::Semifinal,
                MatchFormat::Final,
            ] {
                assert_eq!(arena_case(seed, format), arena_case(seed + 60, format));
            }
        }
    }

    #[test]
    fn ordinary_v19_problems_keep_their_fingerprints() {
        for (seed, format, expected) in [
            (4, MatchFormat::First, 0xf57956e195731747_u64),
            (0, MatchFormat::Semifinal, 0x6074d2586b7cdda4),
            (1, MatchFormat::Final, 0x47fc4d1e034fc009),
        ] {
            let mut case = generate(seed, Some(format)).unwrap();
            assert_eq!(case.generator_version, GENERATOR_VERSION);
            case.generator_version = "hex-udon-v19".into();
            let hash = serde_json::to_vec(&case)
                .unwrap()
                .into_iter()
                .fold(0xcbf29ce484222325_u64, |h, b| {
                    (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
                });
            assert_eq!(
                hash, expected,
                "変更が意図的なら生成バージョンを更新すること"
            );
        }
    }
    #[test]
    fn schedules_start_at_the_minimum_and_cover_gentle_and_steep_growth() {
        for format in [
            MatchFormat::First,
            MatchFormat::Semifinal,
            MatchFormat::Final,
        ] {
            let w = format.parameters().0;
            for days in 4..=10 {
                let (mut gentle, mut steep, mut repeated) = (0, 0, 0);
                for seed in 0..4096 {
                    let steps = make_day_steps(
                        w,
                        days,
                        &mut Random::new(seed, 1, Some(format)),
                        &mut Random::new(seed, 4, Some(format)),
                        GenerationProfile::ResourceLimited,
                    );
                    assert_eq!(steps.len(), days);
                    assert_eq!(steps[0], 2 * w as u32);
                    assert!(
                        steps
                            .iter()
                            .all(|&s| (2 * w as u32..=8 * w as u32).contains(&s))
                    );
                    assert!(steps.windows(2).all(|pair| pair[0] <= pair[1]));
                    let increase = steps.last().unwrap() - steps[0];
                    gentle += usize::from(4 * increase <= steps[0]);
                    steep += usize::from(4 * increase >= 3 * steps[0]);
                    repeated += usize::from(steps.windows(2).any(|pair| pair[0] == pair[1]));
                }
                assert!(
                    gentle > 0 && steep > 0 && repeated > 0,
                    "{format:?} days={days}"
                );
            }
            let limited = (0..4096)
                .filter(|&seed| {
                    generation_profile(seed, format) == GenerationProfile::ResourceLimited
                })
                .count();
            assert!(limited.abs_diff(4096 * RESOURCE_LIMITED_PERCENT / 100) < 4096 / 20);
        }
    }
    #[test]
    fn initial_steps_mix_sample_minimum_with_larger_budgets() {
        for format in [
            MatchFormat::First,
            MatchFormat::Semifinal,
            MatchFormat::Final,
        ] {
            let w = format.parameters().0;
            let (mut minimum, mut above, mut high) = (0, 0, 0);
            for seed in 0..4096 {
                let mut params = Random::new(seed, 1, Some(format));
                let days = params.range(4, 10);
                let steps = make_day_steps(
                    w,
                    days,
                    &mut params,
                    &mut Random::new(seed, 4, Some(format)),
                    generation_profile(seed, format),
                );
                minimum += usize::from(steps[0] == 2 * w as u32);
                above += usize::from(steps[0] > 2 * w as u32);
                high += usize::from(steps[0] >= 7 * w as u32);
                assert!(
                    steps
                        .iter()
                        .all(|&s| (2 * w as u32..=8 * w as u32).contains(&s))
                );
                assert!(steps.windows(2).all(|pair| pair[0] <= pair[1]));
            }
            assert!(
                (4096 * 3 / 4..=4096 * 17 / 20).contains(&minimum),
                "{format:?}: minimum={minimum}"
            );
            assert!(
                (4096 * 3 / 20..=4096 / 4).contains(&above) && high > 0,
                "{format:?}: above={above}, high={high}"
            );
        }
    }
    #[test]
    fn schedules_rarely_stagnate_and_five_day_growth_remains_varied() {
        const COUNT: usize = 4096;
        for format in [
            MatchFormat::First,
            MatchFormat::Semifinal,
            MatchFormat::Final,
        ] {
            let w = format.parameters().0;
            let (mut flat, mut small_growth) = (0, 0);
            let mut five_day_growth = Vec::new();
            for seed in 3..3 + COUNT as u64 {
                let mut params = Random::new(seed, 1, Some(format));
                let days = params.range(4, 10);
                let steps = make_day_steps(
                    w,
                    days,
                    &mut params,
                    &mut Random::new(seed, 4, Some(format)),
                    generation_profile(seed, format),
                );
                let increase = steps.last().unwrap() - steps[0];
                flat += usize::from(increase == 0);
                small_growth += usize::from(10 * increase <= steps[0]);
                if days == 5 {
                    five_day_growth.push(100 * increase / steps[0]);
                }
            }
            // Guard against the former concentration near zero, while allowing
            // occasional flat schedules and both gentle and steep growth.
            assert!(flat < COUNT / 20, "{format:?}: flat={flat}");
            assert!(
                small_growth < COUNT / 10,
                "{format:?}: small={small_growth}"
            );
            five_day_growth.sort_unstable();
            let median = five_day_growth[five_day_growth.len() / 2];
            assert!((25..=100).contains(&median), "{format:?}: median={median}");
            assert!(five_day_growth.iter().any(|&growth| growth <= 25));
            assert!(five_day_growth.iter().any(|&growth| growth >= 75));
        }
    }
    #[test]
    fn spread_spots_include_edges_without_forcing_a_fixed_spacing() {
        for format in [
            MatchFormat::First,
            MatchFormat::Semifinal,
            MatchFormat::Final,
        ] {
            let w = format.parameters().0;
            let (mut adjacent, mut separated) = (0, 0);
            for seed in 0..512 {
                let spots = spread_spots(w, &mut Random::new(seed, 7, Some(format)));
                assert_eq!(spots.len(), w);
                let unique = spots
                    .iter()
                    .copied()
                    .collect::<std::collections::HashSet<_>>();
                assert_eq!(unique.len(), w);
                assert!(spots.iter().all(|&p| p < w * w));
                let edge_count = spots
                    .iter()
                    .filter(|&&p| p % w == 0 || p % w == w - 1 || p / w == 0 || p / w == w - 1)
                    .count();
                assert!((w / 4..=w / 2).contains(&edge_count));
                let nearest = spots
                    .iter()
                    .enumerate()
                    .flat_map(|(i, &a)| spots[i + 1..].iter().map(move |&b| hex_distance(a, b, w)))
                    .min()
                    .unwrap();
                adjacent += usize::from(nearest == 1);
                separated += usize::from(nearest >= 2);
            }
            assert!(adjacent > 0 && separated > 0, "{format:?}");
        }
        let map = Map {
            width: 8,
            height: 8,
            cells: vec![vec![0; 8]; 8],
        };
        for a in 0..map.len() {
            assert_eq!(hex_distance(a, a, 8), 0);
            for b in map.neighbors(a) {
                assert_eq!(hex_distance(a, b, 8), 1);
                assert_eq!(hex_distance(b, a, 8), 1);
            }
        }
    }
    #[test]
    fn width_rule_allows_a_junction_but_not_a_thick_patch() {
        let mut map = Map {
            width: 4,
            height: 4,
            cells: vec![vec![0; 4]; 4],
        };
        for p in [1, 5] {
            map.set(p, 1);
        }
        assert!(thin_addition(&map, 6));
        map.set(6, 1);
        assert!(!thin_addition(&map, 9));
    }
    #[test]
    fn sample_seed_format_pairs_match_for_automatic_and_explicit_selection() {
        for (seed, (json, format)) in SAMPLES
            .iter()
            .zip([
                MatchFormat::First,
                MatchFormat::Semifinal,
                MatchFormat::Final,
            ])
            .enumerate()
        {
            let case = generate(seed as u64, None).unwrap();
            let mut expected: serde_json::Value = serde_json::from_str(json).unwrap();
            expected["players"] = 1.into();
            assert_eq!(serde_json::to_value(&case.problem).unwrap(), expected);
            assert_eq!(case.format, format);
            assert_eq!(case.seed, seed.to_string());
            assert_eq!(case.generator_version, GENERATOR_VERSION);
            case.problem.validate().unwrap();
            assert_eq!(case, generate(seed as u64, None).unwrap());
            for explicit in [
                MatchFormat::First,
                MatchFormat::Semifinal,
                MatchFormat::Final,
            ] {
                let generated = generate(seed as u64, Some(explicit)).unwrap();
                assert_eq!(generated.format, explicit);
                if explicit == format {
                    assert_eq!(generated, case);
                } else {
                    assert_ne!(generated.problem, case.problem);
                }
            }
        }
    }
    #[test]
    fn terrain_components_do_not_merge_into_large_masses() {
        for format in [
            MatchFormat::First,
            MatchFormat::Semifinal,
            MatchFormat::Final,
        ] {
            let mut cell_weighted_sizes = [0.0; 3];
            let mut cases = 0;
            for seed in (3..33).filter(|&seed| arena_case(seed, format).is_none()) {
                cases += 1;
                let map = generate(seed, Some(format)).unwrap().problem.map;
                for (index, terrain) in [0, 2, 3].into_iter().enumerate() {
                    let mut seen = vec![false; map.len()];
                    let (mut cells, mut squares) = (0, 0);
                    for start in 0..map.len() {
                        if seen[start] || map.terrain(start) != terrain {
                            continue;
                        }
                        seen[start] = true;
                        let mut stack = vec![start];
                        let mut size = 0;
                        while let Some(p) = stack.pop() {
                            size += 1;
                            for q in map.neighbors(p) {
                                if !seen[q] && map.terrain(q) == terrain {
                                    seen[q] = true;
                                    stack.push(q);
                                }
                            }
                        }
                        cells += size;
                        squares += size * size;
                    }
                    cell_weighted_sizes[index] += squares as f64 / cells as f64;
                }
            }
            // Broad regression guard on the final map, not the growth-unit size.
            for mean in cell_weighted_sizes.map(|sum| sum / f64::from(cases)) {
                assert!((2.0..12.0).contains(&mean), "{format:?}: {mean}");
            }
        }
    }
    #[test]
    fn maps_vary_terrain_proportions_without_surrounding_every_spot_with_roads() {
        for format in [
            MatchFormat::First,
            MatchFormat::Semifinal,
            MatchFormat::Final,
        ] {
            let (mut low, mut high) = ([100; 4], [0; 4]);
            let (mut entrances, mut spots, mut single, mut multiple) = (0, 0, 0, 0);
            for seed in (3..123).filter(|&seed| arena_case(seed, format).is_none()) {
                let p = generate(seed, Some(format)).unwrap().problem;
                for t in 0..4 {
                    let count = (0..p.map.len())
                        .filter(|&i| p.map.terrain(i) == t as u8)
                        .count();
                    let percent = count * 100 / p.map.len();
                    low[t] = low[t].min(percent);
                    high[t] = high[t].max(percent);
                }
                for spot in p.spots {
                    let count = p
                        .map
                        .neighbors(spot.pos)
                        .filter(|&q| p.map.terrain(q) == 1)
                        .count();
                    entrances += count;
                    spots += 1;
                    single += usize::from(count == 1);
                    multiple += usize::from(count >= 3);
                }
            }
            // Cover both lower and higher terrain shares observed in samples,
            // and retain occasional multiple entrances rather than a hard cap.
            for t in 0..4 {
                assert!(
                    low[t] <= 23 && high[t] >= 28,
                    "{format:?}: {low:?}..{high:?}"
                );
            }
            assert!(single > 0 && multiple > 0, "{format:?}");
            assert!(
                12 * spots <= 10 * entrances && 10 * entrances <= 19 * spots,
                "{format:?}: {entrances}/{spots}"
            );
        }
    }
    #[test]
    fn generated_cases_obey_all_formats_and_reproduce() {
        for f in [
            MatchFormat::First,
            MatchFormat::Semifinal,
            MatchFormat::Final,
        ] {
            let (w, a, low, high, _) = f.parameters();
            let (mut shortest, mut longest) = (u32::MAX, 0);
            for seed in 0..120 {
                let c = generate(seed, Some(f)).unwrap();
                let p = &c.problem;
                p.validate().unwrap();
                assert_eq!(p.map.width, w);
                assert_eq!(p.agents.len(), a);
                assert_eq!(p.spots.len(), w);
                let k = p.spots.iter().map(|s| s.brand).max().unwrap() + 1;
                assert!((low..=high).contains(&k));
                if arena_case(seed, f).is_some() {
                    assert_eq!(c, generate(seed, Some(f)).unwrap());
                    continue;
                }
                for t in 0..4 {
                    assert!((0..p.map.len()).any(|i| p.map.terrain(i) == t));
                }
                if seed >= 3 || f != resolve_format(seed, None) {
                    if generation_profile(seed, f) == GenerationProfile::ResourceLimited {
                        assert_eq!(p.day_steps[0], 2 * w as u32);
                        assert!(p.fuel_limits <= 5 * p.day_steps[0] / 4);
                        assert!(p.busy_threshold <= 2 && p.jammed_threshold <= 4);
                    }
                    shortest = shortest.min(*p.day_steps.iter().min().unwrap());
                    longest = longest.max(*p.day_steps.iter().max().unwrap());
                    for spot in &p.spots {
                        assert!(
                            p.map.neighbors(spot.pos).any(|q| p.map.terrain(q) == 1),
                            "unconnected spot: {seed} {f:?} {}",
                            spot.pos
                        );
                    }
                    for i in (0..p.map.len()).filter(|&i| p.map.terrain(i) == 1) {
                        assert!(thin_addition(&p.map, i), "thick road: {seed} {f:?} {i}");
                    }
                    for t in 0..4 {
                        let n = (0..p.map.len()).filter(|&i| p.map.terrain(i) == t).count();
                        assert!(n * 100 >= p.map.len() * 15 && n * 100 <= p.map.len() * 38);
                    }
                }
                assert!(
                    p.day_steps
                        .iter()
                        .all(|&s| (2 * w as u32..=8 * w as u32).contains(&s))
                );
                assert!(
                    p.day_steps.windows(2).all(|days| days[0] <= days[1]),
                    "decreasing steps: {seed} {f:?} {:?}",
                    p.day_steps
                );
                assert!((p.day_steps[0]..=p.day_steps[0] * 3).contains(&p.fuel_limits));
                assert_eq!(c, generate(seed, Some(f)).unwrap());
            }
            // Starting at 2W still permits gentle schedules and schedules whose
            // final day reaches at least three times the initial budget.
            assert!(shortest <= 3 * w as u32, "{f:?}: shortest={shortest}");
            assert!(longest >= 6 * w as u32, "{f:?}: longest={longest}");
        }
        for seed in [0, 1, 2, 3, 4, 5, 42, u64::MAX] {
            let c = generate(seed, None).unwrap();
            assert_eq!(c, generate(seed, Some(c.format)).unwrap());
        }
    }
}
