//! Independent integer shortest-table producer for a future shared Graph.
//! Importing this crate never loads OpenCL. Only Producer::spawn may do so,
//! after its caller-provided startup gate and singleton cpuset verification.
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

pub mod producer;
mod opencl;

pub const ROUTE_MODES: usize = 6;
pub const MODES: usize = 7;
pub const NO_CELL: u16 = u16::MAX;
pub const INF: i32 = i32::MAX;
pub const MAX_CELLS: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
fn fail(message: impl Into<String>) -> Error { Error(message.into()) }

/// Neighbors are six ordered cell numbers; NO_CELL means no neighbor.
/// Input order, including duplicate target cells, is preserved exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub neighbors: Vec<[u16; 6]>,
    pub passable: Vec<u8>,
    pub durations: Vec<i32>,
    pub costs: Vec<i32>,
    pub shops: Vec<u8>,
    pub targets: Vec<u16>,
}
impl Prepared {
    pub fn cells(&self) -> usize { self.neighbors.len() }
    pub fn entries(&self) -> usize { self.cells() * self.targets.len() }
    pub fn validate(&self) -> Result<()> {
        let n = self.cells();
        if !(1..=MAX_CELLS).contains(&n) || self.targets.is_empty() || self.targets.len() > MAX_CELLS + 64 {
            return Err(fail("input shape exceeds finite table limits"));
        }
        if [self.passable.len(), self.durations.len(), self.costs.len(), self.shops.len()].iter().any(|&v| v != n) {
            return Err(fail("cell array length mismatch"));
        }
        for y in 0..n {
            if self.passable[y] > 1 || self.shops[y] > 1 || !(1..=4).contains(&self.durations[y]) || !(1..=2).contains(&self.costs[y]) {
                return Err(fail(format!("invalid terrain/traffic value at {y}")));
            }
            for (direction, &x) in self.neighbors[y].iter().enumerate() {
                if x == NO_CELL { continue; }
                let x = usize::from(x);
                if x >= n || x == y || self.neighbors[y][..direction].contains(&(x as u16)) {
                    return Err(fail(format!("invalid/duplicate neighbor at {y}")));
                }
                if !self.neighbors[x].contains(&(y as u16)) {
                    return Err(fail("reverse shortest table requires symmetric adjacency"));
                }
            }
        }
        if self.targets.iter().any(|&t| usize::from(t) >= n || self.passable[usize::from(t)] == 0) {
            return Err(fail("target is outside the passable map"));
        }
        Ok(())
    }
    pub fn weight(&self, mode: usize, cell: usize) -> i32 {
        if mode == ROUTE_MODES { return self.costs[cell]; }
        let a = [100, 100, 10, 100, 100, 100][mode];
        let b = [3, 100, 100, 3, 3, 3][mode];
        let reward = if self.shops[cell] != 0 { [0, 0, 0, 20, 80, 160][mode] } else { 0 };
        (self.durations[cell] * a + self.costs[cell] * b - reward).max(1)
    }
}

/// Layout: distance[(mode * targets + target_index) * cells + cell].
/// next has the same layout for only the six route modes.
/// Target, blocked, and unreachable next entries are zero, matching C007.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tables {
    pub distances: Vec<i32>,
    pub next: Vec<u16>,
}
impl Tables {
    pub fn verify(self, input: Arc<Prepared>) -> Result<VerifiedTables> {
        input.validate()?;
        let n = input.cells();
        if self.distances.len() != MODES * input.entries() || self.next.len() != ROUTE_MODES * input.entries() {
            return Err(fail("output shape mismatch"));
        }
        for mode in 0..MODES {
            for (target_index, &target) in input.targets.iter().enumerate() {
                let base = (mode * input.targets.len() + target_index) * n;
                let distance = &self.distances[base..base+n];
                for y in 0..n {
                    let mut best = INF;
                    let mut chosen = 0u16;
                    if input.passable[y] != 0 && y != usize::from(target) {
                        for &x in &input.neighbors[y] {
                            if x == NO_CELL || input.passable[usize::from(x)] == 0 { continue; }
                            let dx = distance[usize::from(x)];
                            if dx == INF { continue; }
                            if dx < 0 { return Err(fail("negative distance")); }
                            let candidate = dx.checked_add(input.weight(mode, y)).ok_or_else(||fail("distance overflow"))?;
                            if candidate < best || (candidate == best && x > chosen) {
                                best = candidate; chosen = x;
                            }
                        }
                    }
                    if y == usize::from(target) { best = 0; }
                    if distance[y] != best {
                        return Err(fail(format!("Bellman equation mismatch mode={mode} target={target_index} cell={y}")));
                    }
                    if mode < ROUTE_MODES && self.next[base+y] != chosen {
                        return Err(fail(format!("deterministic next mismatch mode={mode} target={target_index} cell={y}")));
                    }
                }
            }
        }
        Ok(VerifiedTables { input, tables: self })
    }
}

#[derive(Debug)]
pub struct VerifiedTables { input: Arc<Prepared>, tables: Tables }
impl VerifiedTables {
    pub fn input(&self) -> &Prepared { &self.input }
    pub fn raw(&self) -> &Tables { &self.tables }
    /// One tree DP per route mode/target, not one walk per source path.
    /// All scalars exclude target departure cost and match C007's recurrence.
    pub fn scalars(&self) -> Result<Scalars> { self.scalars_until(None) }
    /// C009: CPU scalar work shares the absolute GPU attempt deadline.
    pub fn scalars_until(&self, deadline: Option<Instant>) -> Result<Scalars> {
        let check = || if deadline.is_some_and(|d| Instant::now() >= d) {
            Err(fail("scalar deadline elapsed"))
        } else { Ok(()) };
        check()?;
        let p = &self.input;
        let n = p.cells();
        let len = ROUTE_MODES * p.entries();
        let mut out = Scalars { times: vec![0;len], fuels: vec![0;len], lengths: vec![0;len], fingerprints: vec![0;len] };
        for mode in 0..ROUTE_MODES {
            for (target_index, &target) in p.targets.iter().enumerate() {
                let base = (mode * p.targets.len() + target_index) * n;
                let mut done = vec![false;n]; done[usize::from(target)] = true;
                let mut stack = Vec::with_capacity(n);
                check()?;
                for start in 0..n {
                    if start % 64 == 0 { check()?; }
                    if done[start] || self.tables.distances[base+start] == INF { continue; }
                    let mut y = start;
                    while !done[y] {
                        if stack.len() >= n { return Err(fail("next tree cycle")); }
                        stack.push(y);
                        let x = usize::from(self.tables.next[base+y]);
                        if x >= n || self.tables.distances[base+x] >= self.tables.distances[base+y] {
                            return Err(fail("next does not strictly decrease distance"));
                        }
                        y = x;
                    }
                    while let Some(y) = stack.pop() {
                        let x = usize::from(self.tables.next[base+y]);
                        out.times[base+y] = p.durations[y] + out.times[base+x];
                        out.fuels[base+y] = p.costs[y] + out.fuels[base+x];
                        out.lengths[base+y] = out.lengths[base+x].checked_add(1).ok_or_else(||fail("route length overflow"))?;
                        out.fingerprints[base+y] = (out.fingerprints[base+x] ^ (x as u64).wrapping_add(0x9e3779b185ebca87)).rotate_left(17).wrapping_mul(0xc2b2ae3d27d4eb4f);
                        done[y] = true;
                    }
                }
            }
        }
        check()?;
        Ok(out)
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct Scalars {
    pub times: Vec<i32>,
    pub fuels: Vec<i32>,
    pub lengths: Vec<u16>,
    pub fingerprints: Vec<u64>,
}

/// Independent strict-update heap reference; it never touches a GPU driver.
pub fn cpu_reference(input: &Prepared) -> Result<Tables> {
    input.validate()?;
    let n = input.cells();
    let mut out = Tables { distances: vec![INF;MODES*input.entries()], next: vec![0;ROUTE_MODES*input.entries()] };
    for mode in 0..MODES {
        for (target_index, &target) in input.targets.iter().enumerate() {
            let base = (mode*input.targets.len()+target_index)*n;
            out.distances[base+usize::from(target)] = 0;
            let mut heap = BinaryHeap::new(); heap.push((Reverse(0),usize::from(target)));
            while let Some((Reverse(cost),x)) = heap.pop() {
                if out.distances[base+x] != cost { continue; }
                for &y in &input.neighbors[x] {
                    if y == NO_CELL || input.passable[usize::from(y)] == 0 { continue; }
                    let y = usize::from(y);
                    let candidate = cost + input.weight(mode,y);
                    if candidate < out.distances[base+y] {
                        out.distances[base+y] = candidate;
                        if mode < ROUTE_MODES { out.next[base+y] = x as u16; }
                        heap.push((Reverse(candidate),y));
                    }
                }
            }
        }
    }
    Ok(out)
}

/// CPU simulation of the kernel's synchronous integer relaxation, for tests.
/// Comparison with cpu_reference checks algorithm/tie equivalence, not speed.
pub fn jacobi_reference(input: &Prepared) -> Result<Tables> {
    input.validate()?;
    let n=input.cells();
    let mut out=Tables { distances:vec![INF;MODES*input.entries()], next:vec![0;ROUTE_MODES*input.entries()] };
    for mode in 0..MODES { for (ti,&target) in input.targets.iter().enumerate() {
        let base=(mode*input.targets.len()+ti)*n;
        let mut a=vec![INF;n];a[usize::from(target)]=0;
        let mut b=a.clone();let mut converged=false;
        for _ in 0..n {
            let mut changed=false;
            for y in 0..n {
                let mut best=a[y];
                if input.passable[y]!=0 && y!=usize::from(target) {
                    for &x in &input.neighbors[y] { if x!=NO_CELL && input.passable[usize::from(x)]!=0 && a[usize::from(x)]!=INF {
                        best=best.min(a[usize::from(x)]+input.weight(mode,y));
                    }}
                }
                b[y]=best;changed|=best!=a[y];
            }
            std::mem::swap(&mut a,&mut b);
            if !changed {converged=true;break;}
        }
        if !converged {return Err(fail("Jacobi failed to converge within cell count"));}
        for y in 0..n {
            out.distances[base+y]=a[y];
            if mode<ROUTE_MODES && input.passable[y]!=0 && y!=usize::from(target) && a[y]!=INF {
                out.next[base+y]=input.neighbors[y].iter().copied().filter(|&x|x!=NO_CELL && input.passable[usize::from(x)]!=0 && a[usize::from(x)]!=INF && a[usize::from(x)]+input.weight(mode,y)==a[y]).max().ok_or_else(||fail("shortest successor missing"))?;
            }
        }
    }}
    Ok(out)
}

