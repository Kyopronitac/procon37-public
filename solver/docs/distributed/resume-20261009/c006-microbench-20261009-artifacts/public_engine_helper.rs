use std::{env,fs,path::Path};
use tester::{engine::{Game,validate_kinds},model::{Request,Response,Plan}};
fn main()->Result<(),Box<dyn std::error::Error>>{
 let args=env::args().collect::<Vec<_>>();let root=Path::new(&args[2]);
 if args[1]=="generate" {
  fs::create_dir_all(root)?;
  for seed in 0..3 {
   let case=tester::generator::generate_for_players(seed,None,4)?;
   let problem=case.problem;let mut kinds=vec![0;problem.agents.len()];kinds[0]=1;
   let game=Game::new(problem.clone(),&kinds);
   let select=Request{protocol_version:1,request_id:1,phase:"select".into(),time_limit_ms:100,problem:Some(problem.clone()),state:None,steps:None,previous_accepted:None};
   let plan=Request{protocol_version:1,request_id:2,phase:"plan".into(),time_limit_ms:10000,problem:None,state:Some(game.observation(0)),steps:Some(problem.day_steps[0]),previous_accepted:Some(true)};
   fs::write(root.join(format!("seed-{seed}.jsonl")),format!("{}\n{}\n",serde_json::to_string(&select)?,serde_json::to_string(&plan)?))?;
  }
 } else if args[1]=="validate" {
  let requests=fs::read_to_string(root)?;let request:Request=serde_json::from_str(requests.lines().next().unwrap())?;
  let problem=request.problem.unwrap();let responses=fs::read_to_string(&args[3])?;
  let mut responses=responses.lines().map(serde_json::from_str::<Response>);
  let select=responses.next().unwrap()?;let response=responses.next().unwrap()?;
  assert_eq!(select.request_id,1);assert_eq!(response.request_id,2);
  let kinds=validate_kinds(&select.answer,problem.agents.len()).map_err(|e|format!("{e:?}"))?;
  assert_eq!(kinds.iter().position(|&k|k==1),Some(0));
  let plan:Plan=serde_json::from_value(response.answer)?;let mut game=Game::new(problem,&kinds);
  game.play_day(plan).map_err(|e|format!("{e:?}"))?;
  println!("{}",serde_json::json!({"verdict":"AC","score":game.score}));
 }else{panic!("expected generate or validate")}
 Ok(())
}
