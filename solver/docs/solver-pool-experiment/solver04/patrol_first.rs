//! Generate broad patrol tours first, then fit a real supplier to them.
use super::{Candidate, Network, Solver, Supply, escort};
use tester::model::Agent;

pub(super) fn build(
    solver: &mut Solver,
    agents: &[Agent],
    net: &Network,
    source: &Candidate,
    day: usize,
    steps: u16,
) -> Candidate {
    // On small teams, keeping part of the incumbent avoids making every
    // patrol depend on an entirely different supplier schedule at once.
    let partial = agents.len() <= 5 && solver.rng.n(2) == 0;
    build_mode(solver, agents, net, source, day, steps, partial)
}

fn build_mode(
    solver: &mut Solver,
    agents: &[Agent],
    net: &Network,
    source: &Candidate,
    day: usize,
    steps: u16,
    partial: bool,
) -> Candidate {
    solver.tour_stats[9] += 1;
    let mut relaxed = Supply::build(&solver.p, agents, net, &source.support, steps);
    // These refills exist only while generating a proposal. The submitted
    // patrols are retimed and repaired against the actual supplier below.
    for time in 0..=steps {
        if time > 0 {
            relaxed.at[time as usize].fill(true);
        }
        relaxed.next[time as usize].fill(if time < steps { time + 1 } else { u16::MAX });
    }
    let mut charged = agents.to_vec();
    for agent in &mut charged {
        agent.fuel = solver.p.fuel_limits;
    }
    let tag = solver.rng.n(1_000_000);
    let proposal = if partial {
        let mut proposal = source.clone();
        solver.rebuild_subset(&mut proposal, &charged, net, &relaxed, day, steps, tag, 65);
        proposal
    } else {
        solver.build_candidate(
            &charged,
            net,
            &relaxed,
            source.support.clone(),
            day,
            steps,
            tag,
            45,
        )
    };
    let support = escort::optimize(
        &solver.p,
        net,
        agents,
        &proposal,
        steps,
        day + 1 == solver.p.day_steps.len(),
        240,
        &mut solver.rng,
    );
    let supply = Supply::build(&solver.p, agents, net, &support, steps);
    let candidate = solver
        .preserve_patrols(&proposal, agents, net, &supply, support, day, steps)
        .unwrap();
    if candidate.score > source.score {
        solver.tour_stats[10] += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "optional diagnostic over an explicitly selected own replay"]
    fn broad_individual_route_probe() {
        let path = std::env::var("PROCON37_PATROL_REPLAY").unwrap();
        let replay: tester::model::Replay =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let day: usize = std::env::var("PROCON37_PROBE_DAY")
            .unwrap()
            .parse()
            .unwrap();
        let record = &replay.days[day];
        let agents = &record.frames[0].agents;
        let net = Network::new(&replay.case.problem, agents, &record.traffics);
        let mut solver = Solver::new(replay.case.problem.clone(), 91);
        for d in &replay.days[..day] {
            for f in &d.frames {
                for e in &f.events {
                    if let tester::model::Event::Collect { brand, .. } = e {
                        solver.seen |= 1 << brand;
                    }
                }
            }
        }
        let mut source = Candidate {
            plan: record.plan.clone(),
            masks: vec![0; agents.len()],
            end: record.frames.last().unwrap().agents.clone(),
            score: 0.0,
            support: agents
                .iter()
                .enumerate()
                .filter(|(_, a)| a.kind == 1)
                .map(|(a, _)| super::super::SupportRoute {
                    stops: vec![],
                    direct: Some(record.plan[a].clone()),
                })
                .collect(),
        };
        for frame in record.frames.iter().skip(1) {
            for (a, agent) in frame.agents.iter().enumerate() {
                let s = net.spot_at[agent.pos];
                if agent.kind == 0 && s >= 0 {
                    source.masks[a] |= 1 << s;
                }
            }
        }
        let steps = record.steps as u16;
        let supply = Supply::build(&solver.p, agents, &net, &source.support, steps);
        source.score = solver.local_value(&source.masks, &source.end, &net, &supply, day, steps);
        for (a, agent) in agents.iter().enumerate() {
            if agent.kind != 0 {
                continue;
            }
            let mut used = vec![0u8; solver.p.spots.len()];
            let mut brands = 0;
            for (b, &mask) in source.masks.iter().enumerate() {
                if b == a {
                    continue;
                }
                for (s, spot) in solver.p.spots.iter().enumerate() {
                    if mask >> s & 1 != 0 {
                        used[s] += 1;
                        brands |= 1 << spot.brand;
                    }
                }
            }
            let started = std::time::Instant::now();
            let route = solver.cell_route(
                a, agents, &net, &supply, day, steps, &used, brands, 2400, 0, 0,
            );
            let mut candidate = source.clone();
            candidate.plan[a] = route.plan;
            candidate.masks[a] = route.mask;
            candidate.end[a].pos = route.pos;
            candidate.end[a].fuel = route.fuel as u32;
            candidate.score =
                solver.local_value(&candidate.masks, &candidate.end, &net, &supply, day, steps);
            let mut game = tester::engine::Game::new(solver.p.clone(), &replay.kinds);
            game.day = day;
            game.agents = agents.clone();
            game.set_observed_traffic(record.traffics.clone());
            game.play_day(candidate.plan.clone()).unwrap();
            let (bb, food) = solver.statistics(&candidate);
            assert_eq!(game.agents, candidate.end);
            assert_eq!(game.score.daily_brands, bb.count_ones() as usize);
            assert_eq!(game.score.udon, food);
            eprintln!(
                "route_probe day={} car={} delta={:.1} food={}->{} elapsed_ms={}",
                day,
                a,
                candidate.score - source.score,
                solver.statistics(&source).1,
                food,
                started.elapsed().as_millis()
            );
        }
    }
    #[test]
    #[ignore = "optional diagnostic over an explicitly selected own replay"]
    fn compare_repair_variants_on_saved_observations() {
        let path = std::env::var("PROCON37_PATROL_REPLAY").expect("path to an own replay");
        let replay: tester::model::Replay =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut seen = 0u64;
        for (day, record) in replay.days.iter().enumerate() {
            let agents = &record.frames[0].agents;
            let net = Network::new(&replay.case.problem, agents, &record.traffics);
            let mut masks = vec![0u32; agents.len()];
            for frame in record.frames.iter().skip(1) {
                for (a, agent) in frame.agents.iter().enumerate() {
                    let s = net.spot_at[agent.pos];
                    if agent.kind == 0 && s >= 0 {
                        masks[a] |= 1 << s;
                    }
                }
            }
            let support = agents
                .iter()
                .enumerate()
                .filter(|(_, a)| a.kind == 1)
                .map(|(a, _)| super::super::SupportRoute {
                    stops: vec![],
                    direct: Some(record.plan[a].clone()),
                })
                .collect::<Vec<_>>();
            let supply = Supply::build(
                &replay.case.problem,
                agents,
                &net,
                &support,
                record.steps as u16,
            );
            for partial in [false, true] {
                let mut solver = Solver::new(replay.case.problem.clone(), 91 + day as u64);
                solver.seen = seen;
                let mut source = Candidate {
                    plan: record.plan.clone(),
                    score: 0.,
                    masks: masks.clone(),
                    end: record.frames.last().unwrap().agents.clone(),
                    support: support.clone(),
                };
                source.score = solver.local_value(
                    &source.masks,
                    &source.end,
                    &net,
                    &supply,
                    day,
                    record.steps as u16,
                );
                let initial = source.score;
                let before_food = solver.statistics(&source).1;
                let started = std::time::Instant::now();
                let mut improved = 0;
                for _ in 0..10 {
                    let candidate = build_mode(
                        &mut solver,
                        agents,
                        &net,
                        &source,
                        day,
                        record.steps as u16,
                        partial,
                    );
                    let mut check =
                        tester::engine::Game::new(replay.case.problem.clone(), &replay.kinds);
                    check.day = day;
                    check.agents = agents.clone();
                    check.set_observed_traffic(record.traffics.clone());
                    check.play_day(candidate.plan.clone()).unwrap();
                    let (brands, food) = solver.statistics(&candidate);
                    assert_eq!(check.agents, candidate.end);
                    assert_eq!(check.score.daily_brands, brands.count_ones() as usize);
                    assert_eq!(check.score.udon, food);
                    if candidate.score > source.score {
                        source = candidate;
                        improved += 1;
                    }
                }
                eprintln!(
                    "fixed_day={} partial={} improvements={} delta={:.1} food={}->{} elapsed_ms={}",
                    day,
                    partial,
                    improved,
                    source.score - initial,
                    before_food,
                    solver.statistics(&source).1,
                    started.elapsed().as_millis()
                );
            }
            for frame in &record.frames {
                for event in &frame.events {
                    if let tester::model::Event::Collect { brand, .. } = event {
                        seen |= 1 << brand;
                    }
                }
            }
        }
    }
    #[test]
    fn relaxed_proposals_are_repaired_with_real_fuel() {
        let p = tester::model::Problem {
            starts_at: 0,
            day_seconds: vec![60],
            day_steps: vec![14],
            map: tester::model::Map {
                width: 5,
                height: 3,
                cells: vec![vec![0; 5]; 3],
            },
            spots: [(0, 1, 2), (1, 4, 1), (2, 7, 2), (3, 13, 1)]
                .into_iter()
                .map(|(brand, pos, stocks)| tester::model::Spot { brand, pos, stocks })
                .collect(),
            agents: vec![0, 3, 10],
            fuel_limits: 4,
            players: 1,
            busy_threshold: 1,
            jammed_threshold: 3,
        };
        let mut game = tester::engine::Game::new(p.clone(), &[0, 0, 1]);
        game.agents[0].fuel = 1;
        game.agents[1].fuel = 0;
        let agents = game.agents.clone();
        let net = Network::new(&p, &agents, &[]);
        let mut solver = Solver::new(p, 19);
        let source = Candidate {
            plan: game.waiting_plan(),
            score: 0.0,
            masks: vec![0; 3],
            end: agents.clone(),
            support: vec![super::super::SupportRoute {
                stops: vec![],
                direct: Some(vec![-14]),
            }],
        };
        let candidate = build(&mut solver, &agents, &net, &source, 0, 14);
        let (brands, food) = solver.statistics(&candidate);
        game.play_day(candidate.plan).unwrap();
        assert_eq!(game.agents, candidate.end);
        assert_eq!(game.score.daily_brands, brands.count_ones() as usize);
        assert_eq!(game.score.udon, food);
    }
}
