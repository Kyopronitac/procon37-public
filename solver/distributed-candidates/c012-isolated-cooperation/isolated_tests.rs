use super::*;
use tester::model::MatchFormat;

fn setup(format: MatchFormat) -> (Solver, Graph, Observation, Solution) {
    let p = tester::generator::generate(0, Some(format)).unwrap().problem;
    let mut kinds = vec![0; p.agents.len()]; kinds[0] = 1;
    let game = Game::new(p.clone(), &kinds); let obs = game.observation(0);
    let mut solver = Solver::new(p.clone(), 0, 0.0, 2.0, 0.0);
    solver.future_weight = 0.0; solver.reserve_weight = 15.0; solver.game = Some(game);
    let graph = Graph::new(&p, &obs, &[]);
    let solution = solver.initial(&graph, &obs, p.day_steps[0] as i32, None);
    (solver, graph, obs, solution)
}
fn bus(offset: usize, count: usize) -> Arc<CrossBus> {
    CrossBus::new(10, 20, 30, offset, count, Instant::now() + Duration::from_secs(60))
}
fn bind(bus: &Arc<CrossBus>, worker: usize, solver: &mut Solver, graph: &Graph, obs: &Observation) -> Binding {
    solver.reserve_weight = RESERVES[(worker / 2) % GROUPS];
    Binding::from_solver(bus, worker, solver, graph, obs, solver.p.day_steps[0] as i32,
        Instant::now() + Duration::from_secs(50)).unwrap()
}
fn arm(binding: &mut Binding) {
    binding.next_check = Instant::now() - CHECK_INTERVAL;
    binding.last_progress = Instant::now() - STAGNATION - CHECK_INTERVAL;
    binding.observed_best = 0.0;
}
fn publish_groups(bus: &Arc<CrossBus>, solver: &mut Solver, graph: &Graph, obs: &Observation, solution: &Solution) {
    for group in 0..GROUPS {
        let worker = (bus.offset..bus.end).find(|&w| (w / 2) % GROUPS == group && !bus.is_receiver(w)).unwrap();
        let mut binding = bind(bus, worker, solver, graph, obs);
        // Deliberately incomparable scores. No cross-group sorting may depend on them.
        binding.publish((1000 - group * 100) as f64, solution);
    }
}

#[test]
fn receiver_roles_are_sparse_generalized_and_cannot_export_or_read_local_elite() {
    assert_eq!(receiver_worker(0,31),Some(7));
    assert_eq!(receiver_worker(31,15),Some(35));
    assert_eq!(receiver_worker(46,31),Some(49));
    assert_eq!(receiver_worker(6,2),Some(7));
    assert_eq!(receiver_worker(34,2),Some(35));
    assert_eq!(receiver_worker(0,7),None);
    assert_eq!(receiver_worker(8,1),None);
    let (mut s,g,obs,solution)=setup(MatchFormat::First);let a=bus(0,31);
    let mut receiver=bind(&a,7,&mut s,&g,&obs);assert!(receiver.is_receiver());
    receiver.publish(1e20,&solution);assert!(a.state.lock().unwrap().offers.is_empty());
    let mut donor=bind(&a,0,&mut s,&g,&obs);arm(&mut donor);
    assert!(donor.take(0.0,Duration::from_secs(2)).is_empty());
    // This is the exact pool role predicate; a receiver must not get its Job.exchange.
    assert!(a.is_receiver(7));assert!(!a.is_receiver(6));
}

#[test]
fn all_formats_legal_batches_fit_or_chunk_and_preserve_every_group() {
    for format in [MatchFormat::First,MatchFormat::Semifinal,MatchFormat::Final] {
        let (mut s,g,obs,solution)=setup(format);let a=bus(0,31);let b=bus(31,15);
        publish_groups(&a,&mut s,&g,&obs,&solution);
        let mut receiver=bind(&b,35,&mut s,&g,&obs);
        let mut cursor=EliteCursor::new();let mut count=0;
        for _ in 0..GROUPS {
            let Some(packet)=a.outgoing(Some(31),true,&mut cursor) else {break;};
            let size=serde_json::to_vec(&crate::wire::ClientMessage::Elite((*packet).clone())).unwrap().len();
            assert!(size<=ELITE_MAX_BYTES);assert!(packet.offers.len()<=BATCH_OFFERS);
            eprintln!("c012_batch format={} bytes={size} offers={}",format.name(),packet.offers.len());
            count+=packet.offers.len();b.receive((*packet).clone(),Some((0,31)));cursor.mark(&packet);
        }
        assert_eq!(count,GROUPS);assert_eq!(b.state.lock().unwrap().offers.len(),GROUPS);
        let mut groups=Vec::new();
        for _ in 0..GROUPS {
            arm(&mut receiver);
            for offer in receiver.take(0.0,Duration::from_secs(2)) {
                groups.push(offer.group);
                let actual=s.evaluate(&g,&obs,s.p.day_steps[0] as i32,offer.solution(),true,None);
                let mut game=s.game.clone().unwrap();game.play_day(actual.plans).unwrap();
            }
        }
        groups.sort_unstable();assert_eq!(groups,(0..GROUPS).collect::<Vec<_>>());
        // Largest supported per-car routes are bounded too; batches may split.
        {let mut state=a.state.lock().unwrap();for stored in &mut state.offers {
            let mut offer=(*stored.offer).clone();offer.sequence+=1;
            offer.solution.routes=vec![vec![(s.p.spots.len()+2*STOPS)*PATH_MODES-1;64];obs.agents.len()];
            assert!(stored.context.valid_solution(&offer.solution));stored.offer=Arc::new(offer);
        }}
        let mut cursor=EliteCursor::new();let mut count=0;
        for _ in 0..GROUPS {
            let Some(packet)=a.outgoing(None,false,&mut cursor) else {break;};
            let size=serde_json::to_vec(&crate::wire::ServerMessage::Elite((*packet).clone())).unwrap().len();
            assert!(size<=ELITE_MAX_BYTES);count+=packet.offers.len();cursor.mark(&packet);
            eprintln!("c012_max_batch format={} bytes={size} offers={}",format.name(),packet.offers.len());
        }
        assert_eq!(count,GROUPS);
    }
}

#[test]
fn envelope_all_context_fields_and_any_bad_member_reject_whole_batch() {
    let (mut s,g,obs,solution)=setup(MatchFormat::First);let a=bus(0,31);let b=bus(31,15);
    publish_groups(&a,&mut s,&g,&obs,&solution);let mut receiver=bind(&b,35,&mut s,&g,&obs);
    let packet=a.outgoing(Some(31),true,&mut EliteCursor::new()).unwrap();
    assert!(packet.offers.len()>1);
    for kind in 0..22 {
        let mut bad=(*packet).clone();
        match kind {
            0=>bad.context.schema+=1,
            1=>bad.context.problem.players+=1,
            2=>bad.context.agents[1].fuel+=1,
            3=>bad.context.day+=1,
            4=>bad.context.steps-=1,
            5=>bad.context.supplier=1,
            6=>bad.context.seen_brands^=1,
            7=>bad.context.positions.swap(0,1),
            8=>bad.context.targets-=1,
            9=>bad.context.durations[0]+=1,
            10=>bad.context.costs[0]+=1,
            11=>bad.session+=1,
            12=>bad.request+=1,
            13=>bad.digest+=1,
            14=>bad.offers[1].group=7,
            15=>bad.offers[1].worker=7, // isolated receivers never export
            16=>bad.offers[1].origin=31,
            17=>bad.offers[1].solution.hubs[0]=g.targets,
            18=>bad.offers[1].value=f64::NAN,
            19=>bad.offers[1].sequence=0,
            20=>bad.offers[1]=bad.offers[0].clone(),
            21=>bad.context.positions.extend(std::iter::repeat_n(0,20000)),
            _=>unreachable!(),
        }
        b.receive(bad,Some((0,31)));arm(&mut receiver);
        assert!(receiver.take(0.0,Duration::from_secs(2)).is_empty(),"mutation {kind}");
        assert!(b.state.lock().unwrap().offers.is_empty(),"partial acceptance {kind}");
    }
    b.receive((*packet).clone(),Some((0,31)));arm(&mut receiver);
    assert!(!receiver.take(0.0,Duration::from_secs(2)).is_empty());
}

#[test]
fn per_group_sequences_prebind_relay_and_round_robin_do_not_starve_low_values() {
    let (mut s,g,obs,solution)=setup(MatchFormat::First);let a=bus(0,31);let b=bus(31,15);
    publish_groups(&a,&mut s,&g,&obs,&solution);let packet=a.outgoing(Some(31),true,&mut EliteCursor::new()).unwrap();
    b.receive((*packet).clone(),Some((0,31)));
    assert!(b.outgoing(None,true,&mut EliteCursor::new()).is_none());
    let _receiver=bind(&b,35,&mut s,&g,&obs);
    let relay=b.outgoing(Some(46),true,&mut EliteCursor::new()).unwrap();
    assert_eq!(relay.offers.iter().map(|v|(v.origin,v.group,v.sequence)).collect::<Vec<_>>(),packet.offers.iter().map(|v|(v.origin,v.group,v.sequence)).collect::<Vec<_>>());
    assert!(b.outgoing(Some(0),true,&mut EliteCursor::new()).is_none());
    assert!(b.outgoing(None,false,&mut EliteCursor::new()).is_none());
    a.receive((*relay).clone(),None);assert_eq!(a.counters.received.load(Ordering::Relaxed),0);
    b.receive((*packet).clone(),Some((0,31)));assert_eq!(b.state.lock().unwrap().offers.len(),GROUPS);
    let mut newer=(*packet).clone();newer.offers[0].sequence=100;
    b.receive(newer,Some((0,31)));
    let mut other=(*packet).clone();other.offers=vec![other.offers[1].clone()];other.offers[0].sequence=2;
    b.receive(other,Some((0,31))); // A lower sequence in a different group is fresh.
    assert_eq!(b.state.lock().unwrap().seen[1],2);
    let c=bus(46,31);let mut wrong=(*packet).clone();wrong.context.positions.swap(0,1);
    c.receive(wrong,None);let mut cr=bind(&c,49,&mut s,&g,&obs);arm(&mut cr);
    assert!(cr.take(0.0,Duration::from_secs(2)).is_empty());
    c.receive((*packet).clone(),None);arm(&mut cr);assert!(!cr.take(0.0,Duration::from_secs(2)).is_empty());
    // Force one offer per batch by using a large but matching context in this serializer test.
    {let mut state=a.state.lock().unwrap();let mut context=(**state.context.as_ref().unwrap()).clone();
        context.positions.extend(std::iter::repeat_n(0,13000));state.context=Some(Arc::new(context));}
    let mut cursor=EliteCursor::new();let mut groups=Vec::new();
    for _ in 0..GROUPS {
        let Some(packet)=a.outgoing(None,false,&mut cursor) else {break;};
        groups.extend(packet.offers.iter().map(|v|v.group));cursor.mark(&packet);
    }
    groups.sort_unstable();assert_eq!(groups,(0..GROUPS).collect::<Vec<_>>());
    // No single offer fits: optional sharing simply produces no packet.
    {let mut state=a.state.lock().unwrap();let mut context=(**state.context.as_ref().unwrap()).clone();
        context.positions.extend(std::iter::repeat_n(0,20000));state.context=Some(Arc::new(context));}
    assert!(a.outgoing(None,false,&mut EliteCursor::new()).is_none());
}

#[test]
fn timing_objective_close_and_seed_mutation_preserve_incumbent() {
    let (mut s,g,obs,solution)=setup(MatchFormat::First);let a=bus(0,31);let b=bus(31,15);
    publish_groups(&a,&mut s,&g,&obs,&solution);let mut receiver=bind(&b,35,&mut s,&g,&obs);
    let packet=a.outgoing(Some(31),true,&mut EliteCursor::new()).unwrap();b.receive((*packet).clone(),Some((0,31)));
    assert!(receiver.take(0.0,Duration::from_secs(2)).is_empty());
    arm(&mut receiver);assert!(receiver.take(0.0,RESTART_GUARD-Duration::from_millis(1)).is_empty());
    arm(&mut receiver);receiver.last_seed=Some(Instant::now());assert!(receiver.take(0.0,Duration::from_secs(2)).is_empty());
    receiver.last_seed=None;arm(&mut receiver);assert!(!receiver.take(0.0,Duration::from_secs(2)).is_empty());
    arm(&mut receiver);assert!(receiver.take(100.0,Duration::from_secs(2)).is_empty()); // local progress resets stall
    let deadline=Instant::now()+Duration::from_secs(50);
    s.future_weight=0.1;assert!(Binding::from_solver(&a,6,&s,&g,&obs,40,deadline).is_none());
    s.future_weight=0.0;s.selecting=true;assert!(Binding::from_solver(&a,6,&s,&g,&obs,40,deadline).is_none());
    s.selecting=false;s.coverage_weight=1.0;assert!(Binding::from_solver(&a,6,&s,&g,&obs,40,deadline).is_none());
    s.coverage_weight=0.0;s.reserve_weight=12.0;assert!(Binding::from_solver(&a,6,&s,&g,&obs,40,deadline).is_none());
    s.reserve_weight=15.0;receiver.deadline=Instant::now()+Duration::from_millis(100);arm(&mut receiver);
    assert!(receiver.take(100.0,Duration::from_secs(2)).is_empty());
    let mut trial=solution.clone();let incumbent=serde_json::to_vec(&solution).unwrap();
    let before:Vec<_>=trial.routes.iter().map(Vec::len).collect();let mut rng=Rng(123);
    let car=destroy_quarter(&mut trial,s.supplier,&mut rng).unwrap();
    assert_ne!(car,s.supplier);assert_eq!(trial.routes[car].len(),before[car]-before[car].div_ceil(4));
    assert_eq!(trial.hubs,solution.hubs);assert_eq!(trial.departures,solution.departures);
    for a in 0..trial.routes.len(){if a!=car{assert_eq!(trial.routes[a],solution.routes[a]);}}
    let _=s.beam_car(&g,&obs,s.p.day_steps[0] as i32,&mut trial,car,Instant::now()+REPAIR_LIMIT,None);
    let out=s.evaluate(&g,&obs,s.p.day_steps[0] as i32,&trial,true,None);
    let mut game=s.game.clone().unwrap();game.play_day(out.plans).unwrap();
    assert_eq!(serde_json::to_vec(&solution).unwrap(),incumbent);
    b.close();let mut newer=(*packet).clone();newer.offers[0].sequence+=1;b.receive(newer,Some((0,31)));
    arm(&mut receiver);assert!(receiver.take(100.0,Duration::from_secs(2)).is_empty());
    a.close();assert!(a.outgoing(None,true,&mut EliteCursor::new()).is_none());
}
