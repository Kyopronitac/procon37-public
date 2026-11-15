#[path = "solver/planner.rs"]
mod planner;
use std::io::{self, BufRead, Write};
use std::sync::{mpsc, Arc, Mutex};
use tester::model::{Problem, Observation, Request};

const WORKERS: usize = 14;
enum Command {
    Select(u64),
    Plan(planner::Snapshot, Observation, u32, u64, bool, planner::Exchange),
}
struct ResultRow { id:usize, answer:serde_json::Value, value:f64, state:planner::Snapshot }
fn main() {
    let seed=std::env::args().nth(1).and_then(|v|v.parse::<u64>().ok()).unwrap_or(206002);
    let stdin=io::stdin();let mut stdout=io::BufWriter::new(io::stdout());
    let mut workers=Vec::new();let mut handles:Vec<std::thread::JoinHandle<()>>=Vec::new();let (results_tx,results_rx)=mpsc::channel();
    let mut chosen:Option<planner::Snapshot>=None;
    for line in stdin.lock().lines() {
        let Ok(line)=line else{break};let Ok(req)=serde_json::from_str::<Request>(&line)else{continue};
        let started=std::time::Instant::now();
        if req.phase=="select" {
            workers.clear();for handle in handles.drain(..){let _=handle.join();}
            let Some(p):Option<Problem>=req.problem else{continue};
            for id in 0..WORKERS {
                let (tx,rx)=mpsc::channel();workers.push(tx);let result=results_tx.clone();let p=p.clone();
                handles.push(std::thread::Builder::new().name(format!("p1-search-{id}")).spawn(move||{
                    let mut solver=planner::Solver::new(p,seed.wrapping_add((id as u64)*7919),0.0,2.0,0.0);
                    for command in rx {
                        let answer=match command {
                            Command::Select(limit)=>serde_json::json!(solver.select_role(id,limit)),
                            Command::Plan(state,obs,steps,limit,accepted,exchange)=>{
                                solver.restore(&state);solver.share(exchange);
                                serde_json::json!(solver.plan(&obs,steps,limit,accepted))
                            }
                        };
                        if result.send(ResultRow{id,answer,value:solver.value(),state:solver.snapshot()}).is_err(){break;}
                    }
                }).expect("start search worker"));
            }
            let limit=if req.time_limit_ms<=110{req.time_limit_ms}else{req.time_limit_ms.saturating_sub(80)};
            for worker in &workers{worker.send(Command::Select(limit)).unwrap();}
        } else if let (Some(state),Some(obs))=(chosen.as_ref(),req.state) {
            let exchange=Arc::new(Mutex::new(None));
            let limit=if req.time_limit_ms<=110{req.time_limit_ms}else{req.time_limit_ms.saturating_sub(80)};
            for worker in &workers{worker.send(Command::Plan(state.clone(),obs.clone(),req.steps.unwrap_or(1),limit,req.previous_accepted.unwrap_or(true),exchange.clone())).unwrap();}
        } else {continue;}
        let mut best:Option<ResultRow>=None;
        for _ in 0..workers.len(){
            let row=results_rx.recv().unwrap();
            if best.as_ref().is_none_or(|b|row.value>b.value || row.value==b.value&&row.id<b.id){best=Some(row);}
        }
        let Some(best)=best else{continue};
        eprintln!("p1 chosen worker={} score={:.3} phase={} elapsed={}",best.id,best.value,req.phase,started.elapsed().as_millis());
        chosen=Some(best.state);
        let response=serde_json::json!({"requestId":req.request_id,"answer":best.answer});
        if writeln!(stdout,"{}",response).is_err()||stdout.flush().is_err(){break;}
    }
    workers.clear();for handle in handles{let _=handle.join();}
}
