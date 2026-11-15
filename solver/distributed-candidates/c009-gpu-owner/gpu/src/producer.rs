//! Persistent, single-flight GPU service. Caller timeouts never join the driver.
//! CPU fallback belongs to the caller; this service never disguises it as GPU.
use crate::{Prepared,Result,Error,fail};
use crate::opencl::Session;
pub use crate::opencl::{Computed,Metrics};
use std::path::{Path,PathBuf};
use std::sync::{Arc,Mutex,Condvar,mpsc};
use std::sync::atomic::{AtomicBool,Ordering};
use std::thread::{self,JoinHandle};
use std::time::Instant;

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Phase { Starting, Ready, Disabled, Stopping, Stopped }
#[derive(Clone,Debug)]
pub struct Status {pub phase:Phase,pub last_error:Option<Error>}

/// This prototype deliberately supports the actual Linux v1 cpuset layout.
/// An external controller must move only the producer TID before the gate ends.
#[derive(Clone,Debug)]
pub struct Confinement {pub cpu:usize,pub group:PathBuf}
impl Confinement {
    pub fn verify_current(&self)->Result<()> {
        let read=|p:&Path|std::fs::read_to_string(p).map_err(|e|fail(format!("{}: {e}",p.display())));
        let mount=Path::new("/sys/fs/cgroup/cpuset");
        let actual=self.group.canonicalize().map_err(|e|fail(format!("cpuset group: {e}")))?;
        if actual!=self.group || actual.parent()!=Some(mount) {return Err(fail("cpuset must be a direct, canonical child of /sys/fs/cgroup/cpuset"));}
        let mounts=read(Path::new("/proc/self/mountinfo"))?;
        let mounted=mounts.lines().any(|line|{
            let Some((left,right))=line.split_once(" - ") else{return false;};
            let l=left.split_whitespace().collect::<Vec<_>>();let r=right.split_whitespace().collect::<Vec<_>>();
            l.len()>=6&&r.len()==3&&l[3]=="/"&&l[4]=="/sys/fs/cgroup/cpuset"&&r[0]=="cgroup"&&r[2].split(',').any(|s|s=="cpuset")
        });
        if !mounted{return Err(fail("verified v1 cpuset mount unavailable"));}
        let membership=read(Path::new("/proc/thread-self/cgroup"))?;
        let expected=format!("/{}",actual.file_name().and_then(|s|s.to_str()).ok_or_else(||fail("cpuset name is not UTF-8"))?);
        let groups=membership.lines().filter_map(|line|{
            let v=line.splitn(3,':').collect::<Vec<_>>();
            (v.len()==3&&v[1].split(',').any(|s|s=="cpuset")).then(||v[2])
        }).collect::<Vec<_>>();
        if groups!=[expected.as_str()]{return Err(fail("producer thread is outside required cpuset"));}
        let cpu=self.cpu.to_string();
        if read(&actual.join("cpuset.cpus"))?.trim()!=cpu{return Err(fail("GPU cpuset is not the required singleton"));}
        let effective=actual.join("cpuset.effective_cpus");
        if effective.exists()&&read(&effective)?.trim()!=cpu{return Err(fail("effective GPU cpuset differs"));}
        let status=read(Path::new("/proc/thread-self/status"))?;
        let allowed=status.lines().find_map(|line|line.strip_prefix("Cpus_allowed_list:")).map(str::trim);
        if allowed!=Some(cpu.as_str()){return Err(fail("producer thread affinity differs"));}
        let tid=current_tid()?;
        if !read(&actual.join("tasks"))?.split_whitespace().any(|s|s.parse::<u32>().ok()==Some(tid)){return Err(fail("producer TID absent from cpuset tasks"));}
        Ok(())
    }
}
pub fn current_tid()->Result<u32>{
    std::fs::read_link("/proc/thread-self").map_err(|e|fail(format!("thread ID: {e}")))?.file_name().and_then(|s|s.to_str()).and_then(|s|s.parse().ok()).ok_or_else(||fail("invalid /proc/thread-self TID"))
}

#[derive(Clone,Debug)]
pub struct Config {pub device_contains:String,pub confinement:Confinement}
struct Shared {status:Mutex<Status>,changed:Condvar,busy:AtomicBool}
impl Shared {
    fn status(&self)->Status{self.status.lock().unwrap().clone()}
    fn update(&self,phase:Phase,error:Option<Error>){
        let mut state=self.status.lock().unwrap();
        // A caller timeout must not overwrite a producer that already exited.
        if state.phase!=Phase::Stopped{state.phase=phase;}
        if error.is_some(){state.last_error=error;}self.changed.notify_all();
    }
}
enum Work { Compute(Arc<Prepared>,Instant,mpsc::SyncSender<Result<Computed>>), Stop }
trait Backend {fn compute(&mut self,input:Arc<Prepared>)->Result<Computed>;}
impl Backend for Session {fn compute(&mut self,input:Arc<Prepared>)->Result<Computed>{Session::compute(self,input)}}

pub struct Producer {sender:mpsc::SyncSender<Work>,shared:Arc<Shared>,thread:Mutex<Option<JoinHandle<()>>>}
impl Producer {
    /// The gate runs on the producer thread before any OpenCL loader call.
    /// It should notify/wait for the external controller to migrate this TID.
    /// It must be finite; a failed gate disables GPU without loading its driver.
    pub fn spawn<F>(config:Config,gate:F)->Result<Self>
    where F:FnOnce(u32)->Result<()>+Send+'static {
        let constraint=config.confinement.clone();
        Self::spawn_backend(move||{
            gate(current_tid()?)?;
            config.confinement.verify_current()?;
            Session::new(&config.device_contains)
        },move||constraint.verify_current())
    }
    fn spawn_backend<B,I,C>(initialize:I,check:C)->Result<Self>
    where B:Backend+'static,I:FnOnce()->Result<B>+Send+'static,C:Fn()->Result<()>+Send+'static {
        let (sender,receiver)=mpsc::sync_channel(1);
        let shared=Arc::new(Shared{status:Mutex::new(Status{phase:Phase::Starting,last_error:None}),changed:Condvar::new(),busy:AtomicBool::new(false)});
        let state=shared.clone();
        let thread=thread::Builder::new().name("gpu-producer".into()).spawn(move||{
            let run=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
                let mut backend=match initialize(){Ok(b)=>b,Err(e)=>{state.update(Phase::Disabled,Some(e));return;}};
                if state.status().phase!=Phase::Starting{return;}
                state.update(Phase::Ready,None);
                while let Ok(work)=receiver.recv(){
                    let Work::Compute(input,deadline,reply)=work else{break;};
                    if state.status().phase!=Phase::Ready {let _=reply.send(Err(fail("producer disabled/stopping")));break;}
                    if Instant::now()>=deadline {let _=reply.send(Err(fail("GPU request expired before dispatch")));state.update(Phase::Disabled,Some(fail("expired GPU request")));break;}
                    let result=check().and_then(|_|backend.compute(input)).and_then(|out|{check()?;Ok(out)});
                    let error=result.as_ref().err().cloned();
                    let _=reply.send(result);
                    if let Some(error)=error{state.update(Phase::Disabled,Some(error));break;}
                    if state.status().phase!=Phase::Ready{break;}
                }
                // Context and buffers are destroyed HERE, never by caller Drop.
                drop(backend);
            }));
            if run.is_err(){state.update(Phase::Disabled,Some(fail("GPU producer panicked")));}
            state.update(Phase::Stopped,None);
        }).map_err(|e|fail(format!("spawn producer: {e}")))?;
        Ok(Self{sender,shared,thread:Mutex::new(Some(thread))})
    }
    pub fn status(&self)->Status{self.shared.status()}
    /// Does not wait for startup or a busy GPU. The caller chooses CPU fallback.
    /// A timeout permanently disables this producer; no more work is admitted.
    pub fn compute(&self,input:Arc<Prepared>,deadline:Instant)->Result<Computed>{
        input.validate()?;
        if self.status().phase!=Phase::Ready{return Err(fail("GPU is not ready"));}
        if self.shared.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err(fail("GPU already has an in-flight request"));}
        struct Release<'a>(&'a AtomicBool);impl Drop for Release<'_>{fn drop(&mut self){self.0.store(false,Ordering::Release);}}
        let _release=Release(&self.shared.busy);
        if self.status().phase!=Phase::Ready{return Err(fail("GPU disabled before dispatch"));}
        let left=deadline.checked_duration_since(Instant::now()).ok_or_else(||fail("GPU request deadline already passed"))?;
        let (sender,receiver)=mpsc::sync_channel(1);
        self.sender.try_send(Work::Compute(input,deadline,sender)).map_err(|_|fail("GPU queue unavailable"))?;
        let received=receiver.recv_timeout(left);
        if Instant::now()>=deadline || matches!(received,Err(mpsc::RecvTimeoutError::Timeout)) {
            self.shared.update(Phase::Disabled,Some(fail("GPU request deadline elapsed")));
            return Err(fail("GPU request deadline elapsed"));
        }
        received.map_err(|_|fail("GPU producer disconnected"))?
    }
    /// Finite caller wait; never attempts to cancel or join a stuck driver.
    /// On timeout, keep Disabled and let the process owner handle final exit.
    pub fn shutdown(&self,deadline:Instant)->Result<()> {
        if self.status().phase!=Phase::Stopped{self.shared.update(Phase::Stopping,None);let _=self.sender.try_send(Work::Stop);}
        let mut state=self.shared.status.lock().unwrap();
        while state.phase!=Phase::Stopped {
            let Some(left)=deadline.checked_duration_since(Instant::now()) else{
                state.phase=Phase::Disabled;state.last_error=Some(fail("GPU shutdown deadline elapsed"));
                return Err(fail("GPU shutdown deadline elapsed; driver thread left to process owner"));
            };
            state=self.shared.changed.wait_timeout(state,left).unwrap().0;
        }
        drop(state);
        if let Some(handle)=self.thread.lock().unwrap().take(){
            // Stopped is the producer's final action; join only if already done.
            if handle.is_finished(){let _=handle.join();}
        }
        Ok(())
    }
}
impl Drop for Producer {
    fn drop(&mut self){
        if self.status().phase!=Phase::Stopped{self.shared.update(Phase::Stopping,None);}
        let _=self.sender.try_send(Work::Stop);
        // Dropping JoinHandle detaches; the submission path never joins driver.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    struct Blocked(mpsc::Receiver<()>);
    impl Backend for Blocked {fn compute(&mut self,_:Arc<Prepared>)->Result<Computed>{self.0.recv().unwrap();Err(fail("test backend released"))}}
    fn input()->Arc<Prepared>{Arc::new(Prepared{neighbors:vec![[crate::NO_CELL;6]],passable:vec![1],durations:vec![2],costs:vec![1],shops:vec![0],targets:vec![0]})}
    #[test]
    fn timeout_disables_admission_and_shutdown_does_not_join_blocked_backend(){
        let (release,blocked)=mpsc::channel();
        let service=Producer::spawn_backend(move||Ok(Blocked(blocked)),||Ok(())).unwrap();
        let until=Instant::now()+Duration::from_secs(2);
        while service.status().phase==Phase::Starting {assert!(Instant::now()<until);thread::yield_now();}
        let started=Instant::now();
        assert!(service.compute(input(),Instant::now()+Duration::from_millis(25)).is_err());
        assert!(started.elapsed()<Duration::from_secs(1));
        assert_eq!(service.status().phase,Phase::Disabled);
        assert!(service.compute(input(),Instant::now()+Duration::from_secs(1)).is_err());
        assert!(service.shutdown(Instant::now()+Duration::from_millis(20)).is_err());
        release.send(()).unwrap();
        service.shutdown(Instant::now()+Duration::from_secs(2)).unwrap();
        assert_eq!(service.status().phase,Phase::Stopped);
    }
    #[test]
    fn rejected_startup_gate_never_loads_opencl(){
        let service=Producer::spawn(Config{device_contains:"must-not-load".into(),confinement:Confinement{cpu:3,group:PathBuf::from("/does-not-exist")}},|_|Err(fail("intentional startup gate rejection"))).unwrap();
        let until=Instant::now()+Duration::from_secs(2);
        while service.status().phase==Phase::Starting {assert!(Instant::now()<until);thread::yield_now();}
        assert!(service.status().last_error.unwrap().0.contains("intentional startup gate rejection"));
        service.shutdown(until).unwrap();
    }
}
