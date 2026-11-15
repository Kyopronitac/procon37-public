// Included inside planner so the benchmark calls the exact private C009 builders.
// This is the benchmark's source, not an edit to the included C009 candidate.
#[derive(serde::Serialize,Clone,PartialEq,Eq)]
struct OwnerThreadPlacement {tid:u32,name:String,allowed:String,cgroup:String}
fn owner_thread_placement(tid:u32)->Result<OwnerThreadPlacement,String> {
    let root=std::path::PathBuf::from(format!("/proc/self/task/{tid}"));
    let status=std::fs::read_to_string(root.join("status")).map_err(|e|e.to_string())?;
    let allowed=status.lines().find_map(|s|s.strip_prefix("Cpus_allowed_list:")).ok_or("missing affinity")?.trim().to_owned();
    Ok(OwnerThreadPlacement{tid,name:std::fs::read_to_string(root.join("comm")).map_err(|e|e.to_string())?.trim().to_owned(),
        allowed,cgroup:std::fs::read_to_string(root.join("cgroup")).map_err(|e|e.to_string())?})
}
fn owner_cpu_ticks()->Result<(u64,u64),String> {
    let text=std::fs::read_to_string("/proc/self/stat").map_err(|e|e.to_string())?;
    let values=text.rsplit_once(')').ok_or("invalid proc stat")?.1.split_whitespace().collect::<Vec<_>>();
    let value=|i:usize|values.get(i).ok_or("missing proc stat field")?.parse::<u64>().map_err(|_|"invalid CPU ticks");
    Ok((value(11)?+value(12)?,value(13)?+value(14)?))
}
fn owner_path(t:&GraphTables,mode:usize,source:usize,target:usize)->&Path {
    if mode==0{&t.paths[source][target]}else{&t.alternatives[mode-1][source][target]}
}
fn owner_equivalent(a:&GraphTables,b:&GraphTables)->Result<usize,String> {
    if a.positions!=b.positions||a.targets!=b.targets||a.durations!=b.durations||a.costs!=b.costs||a.spot_at!=b.spot_at||a.cell_node!=b.cell_node
        ||a.path_mode_mask!=b.path_mode_mask||a.potential!=b.potential||a.mobility!=b.mobility||a.fuel_to!=b.fuel_to{return Err("Graph auxiliary tables differ".into());}
    let mut checked=0;
    for mode in 0..PATH_MODES{for j in 0..a.targets {
        let at=owner_path(a,mode,0,j).tree.as_ref().unwrap();let bt=owner_path(b,mode,0,j).tree.as_ref().unwrap();
        if at.target!=bt.target||at.next!=bt.next||at.fingerprints!=bt.fingerprints{return Err("RouteTree differs".into());}
        for i in 0..a.positions.len(){let x=owner_path(a,mode,i,j);let y=owner_path(b,mode,i,j);
            if (x.source,x.length,x.time,x.fuel)!=(y.source,y.length,y.time,y.fuel){return Err("Path scalar differs".into());}
            checked+=1;
        }
    }}
    Ok(checked)
}

pub fn run_owner_probe(use_gpu:bool)->Result<(),String> {
    let began=Instant::now();let cpu_started=owner_cpu_ticks()?;
    if use_gpu && std::env::var("PROCON37_C009_GPU").ok().as_deref()!=Some("opencl") {return Err("explicit opencl environment required".into());}
    let main_tid=gpu_tables::producer::current_tid().map_err(|e|e.0)?;
    let main_before=owner_thread_placement(main_tid)?;
    // Idle sentinels verify that thread-specific cgroup joining does not migrate
    // existing search-like threads; they are not an active search workload.
    let mut controls=Vec::new();let mut handles=Vec::new();let mut placements=Vec::new();
    for index in 0..15 {
        let (stop_tx,stop_rx)=mpsc::channel::<()>();let (ready_tx,ready_rx)=mpsc::channel();
        handles.push(thread::Builder::new().name(format!("probe-search-{index}")).spawn(move||{
            let tid=gpu_tables::producer::current_tid().unwrap();let _=ready_tx.send(owner_thread_placement(tid));let _=stop_rx.recv();
        }).map_err(|e|e.to_string())?);
        controls.push(stop_tx);placements.push(ready_rx.recv_timeout(Duration::from_secs(1)).map_err(|e|e.to_string())??);
    }
    println!("{}",serde_json::json!({"event":"probe_start","pid":std::process::id(),"gpu":use_gpu,"mainBefore":main_before,"sentinelsBefore":placements,
        "scope":"idle owner builder; no active search, cache lookup/publication or match scoring"}));
    let initialized=Instant::now();let initialization_cpu_started=owner_cpu_ticks()?;
    let service=if use_gpu{Some(gpu_graph::GraphGpu::from_environment().ok_or("GPU factory did not enable v2")?)}else{None};
    let mut samples=Vec::new();
    let result=(||->Result<(),String>{
        if let Some(service)=&service {
            let deadline=initialized+Duration::from_secs(8);
            loop {
                let state=service.snapshot();
                if state["ready"].as_bool()==Some(true){break;}
                if Instant::now()>=deadline{return Err(format!("startup deadline: {state}"));}
                if state["producerStatus"].as_str().is_some_and(|s|s.contains("Stopped")||s.contains("Disabled")){return Err(format!("startup failed: {state}"));}
                thread::sleep(Duration::from_millis(2));
            }
        }
        let initialization_ms=initialized.elapsed().as_secs_f64()*1000.0;
        if owner_thread_placement(main_tid)?!=main_before{return Err("main placement changed".into());}
        for placement in &placements {if owner_thread_placement(placement.tid)?!=*placement{return Err("idle search sentinel placement changed".into());}}
        let initialization_cpu_ended=owner_cpu_ticks()?;
        println!("{}",serde_json::json!({"event":"producer_ready","initializationWaitMs":initialization_ms,
            "processCpuTicks":initialization_cpu_ended.0-initialization_cpu_started.0,"reapedChildCpuTicks":initialization_cpu_ended.1-initialization_cpu_started.1,
            "mainAndSentinelPlacementUnchanged":true}));
        let mut cpu_estimate=Duration::from_millis(100);
        for seed in 0..3 {
            let problem=tester::generator::generate_for_players(seed,None,4).map_err(|e|e.to_string())?.problem;
            let mut kinds=vec![0;problem.agents.len()];kinds[0]=1;
            let game=Game::new(problem.clone(),&kinds);let mut obs=game.observation(0);
            for (i,traffic) in obs.traffics.iter_mut().enumerate(){traffic.status=((i+seed as usize)%3)as u8;}
            let key=PreparedGraph::new(&problem,&obs,&[]);
            for repetition in 0..2 {
                let mut cpu=None;let mut gpu=None;let mut cpu_ms=0.0;let mut gpu_ms=0.0;let mut fallback=false;
                for stage in 0..2 {
                    let is_gpu=(stage+repetition)%2==1;
                    let before=owner_cpu_ticks()?;let gpu_before=service.as_ref().map(|s|s.snapshot());let timer=Instant::now();
                    if is_gpu {
                        gpu=Some(match &service {
                            Some(service)=>match service.try_build(&key,Instant::now()+Duration::from_secs(3),cpu_estimate){Some(tables)=>tables,None=>{fallback=true;GraphTables::build(&key)}},
                            None=>{fallback=true;GraphTables::build(&key)},
                        });
                        gpu_ms=timer.elapsed().as_secs_f64()*1000.0;
                    }else {
                        cpu=Some(GraphTables::build(&key));let elapsed=timer.elapsed();cpu_ms=elapsed.as_secs_f64()*1000.0;cpu_estimate=cpu_estimate.max(elapsed);
                    }
                    let after=owner_cpu_ticks()?;
                    println!("{}",serde_json::json!({"event":"owner_stage","seed":seed,"repetition":repetition,"stage":stage,"mode":if is_gpu{"gpu-attempt"}else{"cpu"},
                        "wallMs":if is_gpu{gpu_ms}else{cpu_ms},"processCpuTicks":after.0-before.0,"reapedChildCpuTicks":after.1-before.1,"fallback":is_gpu&&fallback,
                        "gpuBefore":gpu_before,"gpuAfter":service.as_ref().map(|s|s.snapshot())}));
                }
                let comparison=Instant::now();let paths=owner_equivalent(cpu.as_ref().unwrap(),gpu.as_ref().unwrap())?;
                let sample=serde_json::json!({"event":"pair","seed":seed,"repetition":repetition,"cells":key.map.len(),"targets":key.targets,"sources":key.positions.len(),
                    "cpuMs":cpu_ms,"gpuIncludingFallbackMs":gpu_ms,"fallback":fallback,"exactGraphMatch":true,"pathScalarsChecked":paths,
                    "extraComparisonMs":comparison.elapsed().as_secs_f64()*1000.0});
                println!("{sample}");samples.push(sample);
            }
        }
        if use_gpu&&samples.iter().any(|s|s["fallback"].as_bool()!=Some(false)){return Err("at least one real GPU owner attempt fell back; no all-GPU success claim".into());}
        Ok(())
    })();
    let stopping=Instant::now();let shutdown_cpu_started=owner_cpu_ticks()?;
    if let Some(service)=&service{service.shutdown(Instant::now()+Duration::from_secs(1));}
    let shutdown_ms=stopping.elapsed().as_secs_f64()*1000.0;
    let shutdown_cpu_ended=owner_cpu_ticks()?;
    let gpu_final=service.as_ref().map(|s|s.snapshot());
    // The frozen GraphGpu API logs shutdown errors but returns (). Require its
    // producer diagnostic to prove a normal join; process exit alone is weaker.
    let normal_shutdown=gpu_final.as_ref().is_none_or(|s|s["producerStatus"].as_str()==Some("Status { phase: Stopped, last_error: None }"));
    let main_after=owner_thread_placement(main_tid)?;
    let sentinels_after=placements.iter().map(|p|owner_thread_placement(p.tid)).collect::<Result<Vec<_>,_>>()?;
    drop(controls);for handle in handles {let _=handle.join();}
    let cpu_ended=owner_cpu_ticks()?;
    println!("{}",serde_json::json!({"event":"probe_end","success":result.is_ok()&&normal_shutdown&&main_after==main_before&&sentinels_after==placements,"error":result.as_ref().err(),"wallMs":began.elapsed().as_secs_f64()*1000.0,
        "processCpuTicks":cpu_ended.0-cpu_started.0,"reapedChildCpuTicks":cpu_ended.1-cpu_started.1,"shutdownMs":shutdown_ms,
        "shutdownProcessCpuTicks":shutdown_cpu_ended.0-shutdown_cpu_started.0,"shutdownReapedChildCpuTicks":shutdown_cpu_ended.1-shutdown_cpu_started.1,"normalShutdown":normal_shutdown,
        "mainAfter":main_after,"sentinelsAfter":sentinels_after,"placementUnchanged":main_after==main_before&&sentinels_after==placements,"gpu":gpu_final}));
    result?;
    if !normal_shutdown{return Err("GPU producer did not finish with normal stopped status".into());}
    if main_after!=main_before||sentinels_after!=placements{return Err("CPU thread placement changed".into());}
    Ok(())
}
