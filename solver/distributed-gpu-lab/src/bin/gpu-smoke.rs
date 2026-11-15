//! Bounded API/driver correctness probe, never a match or a solver benchmark.
use procon37_distributed_gpu_lab::{
    cpu_reference, jacobi_reference, Error, Prepared, Result, Scalars, Tables,
    INF, MODES, NO_CELL, ROUTE_MODES,
};
use procon37_distributed_gpu_lab::producer::{Config, Confinement, Phase, Producer};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const HELP: &str = "gpu-smoke --cpu-only\n\
gpu-smoke --allow-gpu --device NAME --cpu N --cpuset /sys/fs/cgroup/cpuset/NAME\n\
\nRuns deterministic synthetic 16x16, 24x24, 32x32 fixtures, eight targets each.\n\
GPU mode uses one persistent context and exactly two requests per fixture.\n\
It requires an already enforced v1 singleton cpuset, not taskset alone.\n\
Startup deadline 8 s, each GPU request 5 s, GPU phase 25 s, shutdown 1 s.\n\
Use an external finite process monitor to collect and reap driver descendants.\n\
Cold means a fresh process/context, not an emptied driver compilation cache.\n";

enum Options { CpuOnly, Gpu(Config), Help }
fn options(args: &[String]) -> Result<Options> {
    if args == ["--help"] { return Ok(Options::Help); }
    if args == ["--cpu-only"] { return Ok(Options::CpuOnly); }
    if args.len() != 7 || args[0] != "--allow-gpu" || args[1] != "--device"
        || args[3] != "--cpu" || args[5] != "--cpuset" || args[2].is_empty() {
        return Err(Error(format!("explicit mode and exact arguments required\n{HELP}")));
    }
    let cpu = args[4].parse::<usize>().map_err(|_| Error("invalid CPU number".into()))?;
    Ok(Options::Gpu(Config {
        device_contains: args[2].clone(),
        confinement: Confinement { cpu, group: PathBuf::from(&args[6]) },
    }))
}

fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""), '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"), '\r' => out.push_str("\\r"), '\t' => out.push_str("\\t"),
            c if c < '\u{20}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"'); out
}
fn ms(start: Instant) -> f64 { start.elapsed().as_secs_f64() * 1000.0 }

fn fixture(side: usize) -> Prepared {
    let n = side * side;
    let mut p = Prepared {
        neighbors: vec![[NO_CELL; 6]; n], passable: vec![1; n],
        durations: vec![2; n], costs: vec![1; n], shops: vec![0; n], targets: vec![],
    };
    let mut seed = 0x3720_2610_0900_u64 + side as u64;
    for row in 0..side { for column in 0..side {
        let y = row * side + column;
        let left = if row % 2 == 0 { 0 } else { -1 };
        for (d, (dy, dx)) in [(-1,left),(-1,left+1),(0,1),(1,left+1),(1,left),(0,-1)].into_iter().enumerate() {
            let yy = row as isize + dy; let xx = column as isize + dx;
            if yy >= 0 && xx >= 0 && yy < side as isize && xx < side as isize {
                p.neighbors[y][d] = (yy as usize * side + xx as usize) as u16;
            }
        }
        seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
        p.passable[y] = u8::from(y == 0 || (seed >> 12) % 11 != 0);
        p.durations[y] = 1 + (seed % 4) as i32;
        p.costs[y] = 1 + ((seed >> 5) % 2) as i32;
        p.shops[y] = u8::from((seed >> 8) % 7 == 0);
    }}
    let valid = (0..n).filter(|&y| p.passable[y] != 0).collect::<Vec<_>>();
    for i in 0..7 { p.targets.push(valid[i * (valid.len()-1) / 6] as u16); }
    p.targets.push(p.targets[0]); // Exercise ordered duplicate targets.
    p
}

/// Independent full path walk: do not validate the tree DP by calling itself.
fn validate_scalars(p: &Prepared, tables: &Tables, s: &Scalars) -> Result<usize> {
    let n = p.cells(); let mut routes = 0;
    for mode in 0..ROUTE_MODES { for (ti, &target) in p.targets.iter().enumerate() {
        let base = (mode * p.targets.len() + ti) * n;
        for source in 0..n {
            let index = base + source;
            if tables.distances[index] == INF {
                if (s.times[index], s.fuels[index], s.lengths[index], s.fingerprints[index]) != (0,0,0,0) {
                    return Err(Error("unreachable scalar must be zero".into()));
                }
                continue;
            }
            let mut path = Vec::new(); let mut cell = source; let mut time = 0; let mut fuel = 0;
            while cell != usize::from(target) {
                if path.len() >= n { return Err(Error("scalar validation path is cyclic".into())); }
                time += p.durations[cell]; fuel += p.costs[cell];
                cell = usize::from(tables.next[base+cell]); path.push(cell);
            }
            let mut fingerprint = 0u64;
            for &next in path.iter().rev() {
                fingerprint = (fingerprint ^ (next as u64).wrapping_add(0x9e3779b185ebca87))
                    .rotate_left(17).wrapping_mul(0xc2b2ae3d27d4eb4f);
            }
            if (s.times[index], s.fuels[index], usize::from(s.lengths[index]), s.fingerprints[index])
                != (time, fuel, path.len(), fingerprint) {
                return Err(Error(format!("scalar mismatch mode={mode} target={ti} source={source}")));
            }
            routes += 1;
        }
    }}
    Ok(routes)
}

/// Diagnostic equality fingerprint, not a cryptographic artifact identifier.
fn checksum(tables: &Tables) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in tables.distances.iter().flat_map(|v|v.to_le_bytes())
        .chain(tables.next.iter().flat_map(|v|v.to_le_bytes())) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    hash
}

struct Case { side: usize, input: Arc<Prepared>, reference: Tables }
fn prepare(cpu_only: bool) -> Result<Vec<Case>> {
    let mut cases = Vec::new();
    for side in [16, 24, 32] {
        let input = Arc::new(fixture(side)); let started = Instant::now();
        let reference = cpu_reference(&input)?; let heap_ms = ms(started);
        let verified = reference.clone().verify(input.clone())?;
        let scalars = verified.scalars()?;
        let routes = validate_scalars(&input, &reference, &scalars)?;
        let jacobi_ms = if cpu_only {
            let started = Instant::now(); let simulated = jacobi_reference(&input)?;
            if simulated != reference { return Err(Error("CPU Jacobi differs from heap reference".into())); }
            Some(ms(started))
        } else { None };
        println!("{{\"event\":\"cpu_reference\",\"fixture\":\"synthetic-{side}x{side}\",\"targets\":8,\"route_checks\":{routes},\"distance_entries\":{},\"next_entries\":{},\"heap_ms\":{heap_ms:.3},\"jacobi_ms\":{},\"table_fnv1a64\":\"{:016x}\"}}",
            MODES*input.entries(), ROUTE_MODES*input.entries(), jacobi_ms.map(|v|format!("{v:.3}")).unwrap_or("null".into()), checksum(&reference));
        cases.push(Case { side, input, reference });
    }
    Ok(cases)
}

fn gpu(config: Config, cases: &[Case]) -> Result<()> {
    // For this standalone smoke process all threads are deliberately confined.
    // Future solver integration moves only the producer TID inside its gate.
    config.confinement.verify_current()?;
    let started = Instant::now(); let overall = started + Duration::from_secs(25);
    let service = Producer::spawn(config, |_| Ok(()))?;
    let run = (|| -> Result<()> {
        let startup_deadline = started + Duration::from_secs(8);
        loop {
            let status = service.status();
            match status.phase {
                Phase::Ready => break,
                Phase::Starting if Instant::now() < startup_deadline => std::thread::sleep(Duration::from_millis(2)),
                _ => return Err(status.last_error.unwrap_or(Error("GPU startup deadline/state failure".into()))),
            }
        }
        println!("{{\"event\":\"gpu_ready\",\"startup_wait_ms\":{:.3},\"cold_definition\":\"fresh-process-context-driver-cache-preserved\"}}", ms(started));
        for case in cases { for repeat in 0..2 {
            let request_started = Instant::now();
            let deadline = (request_started + Duration::from_secs(5)).min(overall);
            let computed = service.compute(case.input.clone(), deadline)?;
            let caller_ms = ms(request_started);
            if computed.tables.raw() != &case.reference {
                return Err(Error(format!("GPU/heap table mismatch side={} repeat={repeat}", case.side)));
            }
            let scalar_started = Instant::now(); let scalars = computed.tables.scalars()?;
            let scalar_ms = ms(scalar_started);
            let routes = validate_scalars(&case.input, computed.tables.raw(), &scalars)?;
            let m = &computed.metrics;
            println!("{{\"event\":\"gpu_request\",\"fixture\":\"synthetic-{}x{}\",\"repeat\":{repeat},\"device\":{},\"sequence\":{},\"first_after_init\":{},\"context_initialization_ms\":{:.3},\"allocation_ms\":{:.3},\"upload_ms\":{:.3},\"execution_and_read_ms\":{:.3},\"verification_ms\":{:.3},\"caller_ms\":{caller_ms:.3},\"scalar_dp_ms\":{scalar_ms:.3},\"rounds_max\":{},\"device_allocated_bytes\":{},\"route_checks\":{routes},\"exact_table_match\":true,\"table_fnv1a64\":\"{:016x}\"}}",
                case.side, case.side, quoted(&m.device_name), m.request_sequence, m.request_sequence == 1,
                m.initialization_ms, m.allocation_ms, m.upload_ms, m.execution_and_read_ms,
                m.verification_ms, m.rounds_max, m.device_allocated_bytes, checksum(computed.tables.raw()));
        }}
        Ok(())
    })();
    let shutdown_started = Instant::now();
    let cleanup = service.shutdown(shutdown_started + Duration::from_secs(1));
    let status = service.status();
    println!("{{\"event\":\"gpu_shutdown\",\"shutdown_ms\":{:.3},\"completed\":{},\"phase\":{},\"last_error\":{},\"gpu_phase_ms\":{:.3}}}",
        ms(shutdown_started), cleanup.is_ok(), quoted(&format!("{:?}",status.phase)),
        status.last_error.map(|e|quoted(&e.0)).unwrap_or("null".into()), ms(started));
    run?; cleanup
}

fn run() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let config = match options(&args)? {
        Options::Help => { print!("{HELP}"); return Ok(()); }
        Options::CpuOnly => None,
        Options::Gpu(config) => { config.confinement.verify_current()?; Some(config) }
    };
    let started = Instant::now();
    println!("{{\"event\":\"probe_start\",\"pid\":{},\"cpu_only\":{},\"synthetic_fixture_only\":true}}", std::process::id(), config.is_none());
    let cases = prepare(config.is_none())?;
    if let Some(config) = config { gpu(config, &cases)?; }
    println!("{{\"event\":\"probe_complete\",\"wall_ms\":{:.3},\"success\":true}}",ms(started));
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        println!("{{\"event\":\"probe_error\",\"error\":{}}}", quoted(&error.0));
        std::process::exit(1); // Process owner must still reap driver descendants.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_modes_and_json_escaping() {
        assert!(matches!(options(&["--cpu-only".into()]).unwrap(), Options::CpuOnly));
        assert!(options(&[]).is_err());
        assert!(options(&["--allow-gpu".into()]).is_err());
        assert_eq!(quoted("a\n\"\\\u{1}"), "\"a\\n\\\"\\\\\\u0001\"");
    }
    #[test]
    fn deterministic_fixtures_keep_duplicate_targets() {
        for side in [16,24,32] {
            let p = fixture(side); p.validate().unwrap();
            assert_eq!(p, fixture(side)); assert_eq!(p.targets.len(),8);
            assert_eq!(p.targets[0],p.targets[7]);
        }
    }
}
