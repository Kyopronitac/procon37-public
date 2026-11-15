//! Separate finite probe: it includes the validated C009 implementation unchanged.
#[path = "../../../distributed-candidates/c009-gpu-owner/wire.rs"]
mod wire;
#[allow(dead_code)]
mod planner {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../distributed-candidates/c009-gpu-owner/solver/planner.rs"));
    include!("owner_impl.rs");
}

fn main() {
    let args=std::env::args().skip(1).collect::<Vec<_>>();
    let gpu=match args.as_slice() {
        [flag] if flag=="--allow-gpu"=>true,
        [flag] if flag=="--cpu-only"=>false,
        _=>{eprintln!("owner-probe --cpu-only | --allow-gpu (explicit v2 C009 environment required)");std::process::exit(2);}
    };
    if let Err(error)=planner::run_owner_probe(gpu) {
        eprintln!("{}",serde_json::json!({"event":"probe_error","error":error}));
        std::process::exit(1);
    }
}
