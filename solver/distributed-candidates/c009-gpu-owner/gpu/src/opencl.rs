//! Small Linux OpenCL 1.2 dynamic binding. All calls belong to the producer.
//! This module has no initializer and is never entered by CPU-only tests.
use crate::{Prepared, Tables, VerifiedTables, Result, fail, MODES, ROUTE_MODES};
use std::ffi::{c_char, c_void, CStr, CString};
use std::marker::PhantomData;
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

type Raw = *mut c_void;
type Int = i32;
type Uint = u32;
type ContextNotify = Option<unsafe extern "C" fn(*const c_char,*const c_void,usize,*mut c_void)>;
type ProgramNotify = Option<unsafe extern "C" fn(Raw,*mut c_void)>;
#[link(name="dl")]
unsafe extern "C" {
    fn dlopen(filename:*const c_char,flags:Int)->Raw;
    fn dlsym(handle:Raw,name:*const c_char)->Raw;
    fn dlclose(handle:Raw)->Int;
    fn dlerror()->*const c_char;
}
struct Library(Raw);
impl Library {
    fn open()->Result<Self> {
        // RTLD_NOW; RTLD_LOCAL is zero on the supported Linux target.
        let handle=unsafe{dlopen(c"libOpenCL.so.1".as_ptr(),2)};
        if handle.is_null(){Err(fail(format!("OpenCL loader: {}",loader_error())))}else{Ok(Self(handle))}
    }
    fn symbol(&self,name:&str)->Result<Raw> {
        let name=CString::new(name).unwrap();
        let address=unsafe{dlsym(self.0,name.as_ptr())};
        if address.is_null(){Err(fail(format!("OpenCL symbol: {}",loader_error())))}else{Ok(address)}
    }
}
impl Drop for Library {fn drop(&mut self){unsafe{dlclose(self.0);}}}
fn loader_error()->String {unsafe{let p=dlerror();if p.is_null(){"unknown loader error".into()}else{CStr::from_ptr(p).to_string_lossy().into_owned()}}}
fn check(code:Int,operation:&str)->Result<()> {if code==0{Ok(())}else{Err(fail(format!("{operation}: OpenCL error {code}")))}}

macro_rules! bindings {
    ($($name:ident : $ty:ty),+ $(,)?)=>{
        #[allow(non_snake_case)]
        struct Api {$($name:$ty,)+ _library:Library}
        impl Api {
            fn load()->Result<Self>{
                let library=Library::open()?;
                Ok(Self{$($name:unsafe{std::mem::transmute::<Raw,$ty>(library.symbol(stringify!($name))?)},)+_library:library})
            }
        }
    }
}
bindings! {
    clGetPlatformIDs: unsafe extern "C" fn(Uint,*mut Raw,*mut Uint)->Int,
    clGetDeviceIDs: unsafe extern "C" fn(Raw,u64,Uint,*mut Raw,*mut Uint)->Int,
    clGetDeviceInfo: unsafe extern "C" fn(Raw,Uint,usize,*mut c_void,*mut usize)->Int,
    clCreateContext: unsafe extern "C" fn(*const isize,Uint,*const Raw,ContextNotify,*mut c_void,*mut Int)->Raw,
    clCreateCommandQueue: unsafe extern "C" fn(Raw,Raw,u64,*mut Int)->Raw,
    clCreateProgramWithSource: unsafe extern "C" fn(Raw,Uint,*const *const c_char,*const usize,*mut Int)->Raw,
    clBuildProgram: unsafe extern "C" fn(Raw,Uint,*const Raw,*const c_char,ProgramNotify,*mut c_void)->Int,
    clGetProgramBuildInfo: unsafe extern "C" fn(Raw,Raw,Uint,usize,*mut c_void,*mut usize)->Int,
    clCreateKernel: unsafe extern "C" fn(Raw,*const c_char,*mut Int)->Raw,
    clGetKernelWorkGroupInfo: unsafe extern "C" fn(Raw,Raw,Uint,usize,*mut c_void,*mut usize)->Int,
    clCreateBuffer: unsafe extern "C" fn(Raw,u64,usize,*mut c_void,*mut Int)->Raw,
    clSetKernelArg: unsafe extern "C" fn(Raw,Uint,usize,*const c_void)->Int,
    clEnqueueWriteBuffer: unsafe extern "C" fn(Raw,Raw,Uint,usize,usize,*const c_void,Uint,*const Raw,*mut Raw)->Int,
    clEnqueueNDRangeKernel: unsafe extern "C" fn(Raw,Raw,Uint,*const usize,*const usize,*const usize,Uint,*const Raw,*mut Raw)->Int,
    clEnqueueReadBuffer: unsafe extern "C" fn(Raw,Raw,Uint,usize,usize,*mut c_void,Uint,*const Raw,*mut Raw)->Int,
    clReleaseMemObject: unsafe extern "C" fn(Raw)->Int,
    clReleaseKernel: unsafe extern "C" fn(Raw)->Int,
    clReleaseProgram: unsafe extern "C" fn(Raw)->Int,
    clReleaseCommandQueue: unsafe extern "C" fn(Raw)->Int,
    clReleaseContext: unsafe extern "C" fn(Raw)->Int,
}

#[derive(Debug)]
pub struct Metrics {
    pub device_name:String,
    pub initialization_ms:f64,
    pub allocation_ms:f64,
    pub upload_ms:f64,
    /// Dispatch plus blocking reads, not device kernel-only timing.
    pub execution_and_read_ms:f64,
    pub verification_ms:f64,
    pub rounds_max:u32,
    pub request_sequence:u64,
    pub device_allocated_bytes:usize,
}
#[derive(Debug)]
pub struct Computed {pub tables:VerifiedTables,pub metrics:Metrics}

/// Not Send/Sync: never move the initialized context to a search worker.
pub(crate) struct Session {
    api:Api, context:Raw, queue:Raw, program:Raw, kernel:Raw,
    buffers:[Raw;9], capacities:[usize;9], group:usize,
    device_name:String, initialization_ms:f64, sequence:u64,
    _owner_thread:PhantomData<Rc<()>>,
}
impl Session {
    pub(crate) fn new(selector:&str)->Result<Self> {
        let started=Instant::now();
        let api=Api::load()?;
        let mut count=0;
        check(unsafe{(api.clGetPlatformIDs)(0,null_mut(),&mut count)},"platform count")?;
        if count==0 || count>64{return Err(fail("invalid OpenCL platform count"));}
        let mut platforms=vec![null_mut();count as usize];
        check(unsafe{(api.clGetPlatformIDs)(count,platforms.as_mut_ptr(),null_mut())},"platforms")?;
        let mut chosen=None;
        for platform in platforms {
            let mut count=0;
            let code=unsafe{(api.clGetDeviceIDs)(platform,4,0,null_mut(),&mut count)}; // GPU only
            if code == -1 {continue;}
            check(code,"GPU count")?;
            if count>64{return Err(fail("invalid GPU count"));}
            let mut devices=vec![null_mut();count as usize];
            if count==0{continue;}
            check(unsafe{(api.clGetDeviceIDs)(platform,4,count,devices.as_mut_ptr(),null_mut())},"GPUs")?;
            for device in devices {
                let name=device_name(&api,device)?;
                if name.to_lowercase().contains(&selector.to_lowercase()) {chosen=Some((device,name));break;}
            }
            if chosen.is_some(){break;}
        }
        let (device,name)=chosen.ok_or_else(||fail(format!("no GPU matching {selector:?}; CPU devices are never selected")))?;
        let mut little_endian=0u32;
        check(unsafe{(api.clGetDeviceInfo)(device,0x1026,std::mem::size_of::<u32>(),(&mut little_endian as *mut u32).cast(),null_mut())},"device byte order")?;
        if little_endian!=1 || cfg!(target_endian="big"){return Err(fail("prototype requires matching little-endian host and GPU"));}
        let mut s=Self{api,context:null_mut(),queue:null_mut(),program:null_mut(),kernel:null_mut(),buffers:[null_mut();9],capacities:[0;9],group:0,device_name:name,initialization_ms:0.0,sequence:0,_owner_thread:PhantomData};
        let mut error=0;
        s.context=unsafe{(s.api.clCreateContext)(null(),1,&device,None,null_mut(),&mut error)};
        check(error,"create context")?;require_handle(s.context,"context")?;
        s.queue=unsafe{(s.api.clCreateCommandQueue)(s.context,device,0,&mut error)};
        check(error,"create queue")?;require_handle(s.queue,"queue")?;
        let source=include_str!("shortest.cl");let pointer=source.as_ptr().cast::<c_char>();let length=source.len();
        s.program=unsafe{(s.api.clCreateProgramWithSource)(s.context,1,&pointer,&length,&mut error)};
        check(error,"create program")?;require_handle(s.program,"program")?;
        let code=unsafe{(s.api.clBuildProgram)(s.program,1,&device,c"-cl-std=CL1.2".as_ptr(),None,null_mut())};
        if code!=0 {
            let mut length=0;
            unsafe{(s.api.clGetProgramBuildInfo)(s.program,device,0x1183,0,null_mut(),&mut length);}
            let mut bytes=vec![0u8;length.min(1<<20)];
            if !bytes.is_empty(){unsafe{(s.api.clGetProgramBuildInfo)(s.program,device,0x1183,bytes.len(),bytes.as_mut_ptr().cast(),null_mut());}}
            return Err(fail(format!("build kernel: error {code}: {}",String::from_utf8_lossy(&bytes))));
        }
        s.kernel=unsafe{(s.api.clCreateKernel)(s.program,c"build_tables".as_ptr(),&mut error)};
        check(error,"create kernel")?;require_handle(s.kernel,"kernel")?;
        let mut group=0usize;
        check(unsafe{(s.api.clGetKernelWorkGroupInfo)(s.kernel,device,0x11b0,std::mem::size_of::<usize>(),(&mut group as *mut usize).cast(),null_mut())},"kernel group limit")?;
        let mut local=0u64;
        check(unsafe{(s.api.clGetDeviceInfo)(device,0x1023,std::mem::size_of::<u64>(),(&mut local as *mut u64).cast(),null_mut())},"local memory limit")?;
        if group==0 || local < (crate::MAX_CELLS*8+4) as u64{return Err(fail("GPU local memory/workgroup limit insufficient"));}
        s.group=group.min(128);
        s.initialization_ms=started.elapsed().as_secs_f64()*1000.0;
        Ok(s)
    }
    pub(crate) fn compute(&mut self,input:Arc<Prepared>)->Result<Computed> {
        input.validate()?;
        let n=input.cells();let targets=input.targets.len();let entries=input.entries();
        let sizes=[n*12,n,n*4,n*4,n,targets*2,MODES*entries*4,ROUTE_MODES*entries*2,MODES*targets*4];
        let began=Instant::now();
        for (i,&size) in sizes.iter().enumerate() {
            if size>self.capacities[i] {
                let mut error=0;
                let new=unsafe{(self.api.clCreateBuffer)(self.context,1,size,null_mut(),&mut error)};
                check(error,"allocate table buffer")?;require_handle(new,"table buffer")?;
                if !self.buffers[i].is_null(){unsafe{(self.api.clReleaseMemObject)(self.buffers[i]);}}
                self.buffers[i]=new;self.capacities[i]=size;
            }
        }
        let allocation_ms=began.elapsed().as_secs_f64()*1000.0;
        let began=Instant::now();
        let pointers=[input.neighbors.as_ptr().cast::<c_void>(),input.passable.as_ptr().cast(),input.durations.as_ptr().cast(),input.costs.as_ptr().cast(),input.shops.as_ptr().cast(),input.targets.as_ptr().cast()];
        for (i,&pointer) in pointers.iter().enumerate() {
            check(unsafe{(self.api.clEnqueueWriteBuffer)(self.queue,self.buffers[i],1,0,sizes[i],pointer,0,null(),null_mut())},"upload graph")?;
        }
        let upload_ms=began.elapsed().as_secs_f64()*1000.0;
        let n32=n as u32;let targets32=targets as u32;
        self.arg(0,&n32)?;self.arg(1,&targets32)?;
        for i in 0..9{self.arg(2+i as u32,&self.buffers[i])?;}
        for (index,bytes) in [(11,n*4),(12,n*4),(13,4)] {
            check(unsafe{(self.api.clSetKernelArg)(self.kernel,index,bytes,null())},"local kernel arg")?;
        }
        let mut output=Tables{distances:vec![0;MODES*entries],next:vec![0;ROUTE_MODES*entries]};
        let mut rounds=vec![0u32;MODES*targets];
        let began=Instant::now();let global=MODES*targets*self.group;
        check(unsafe{(self.api.clEnqueueNDRangeKernel)(self.queue,self.kernel,1,null(),&global,&self.group,0,null(),null_mut())},"enqueue shortest tables")?;
        for (slot,pointer) in [(6,output.distances.as_mut_ptr().cast::<c_void>()),(7,output.next.as_mut_ptr().cast()),(8,rounds.as_mut_ptr().cast())] {
            check(unsafe{(self.api.clEnqueueReadBuffer)(self.queue,self.buffers[slot],1,0,sizes[slot],pointer,0,null(),null_mut())},"read shortest tables")?;
        }
        let execution_and_read_ms=began.elapsed().as_secs_f64()*1000.0;
        if rounds.iter().any(|&v|v==0||v>n as u32){return Err(fail("GPU relaxation failed to converge"));}
        let began=Instant::now();let tables=output.verify(input)?;
        let verification_ms=began.elapsed().as_secs_f64()*1000.0;
        self.sequence+=1;
        Ok(Computed{tables,metrics:Metrics{device_name:self.device_name.clone(),initialization_ms:self.initialization_ms,allocation_ms,upload_ms,execution_and_read_ms,verification_ms,rounds_max:*rounds.iter().max().unwrap(),request_sequence:self.sequence,device_allocated_bytes:self.capacities.iter().sum()}})
    }
    fn arg<T>(&self,index:u32,value:&T)->Result<()> {
        check(unsafe{(self.api.clSetKernelArg)(self.kernel,index,std::mem::size_of::<T>(),(value as *const T).cast())},"kernel arg")
    }
}
impl Drop for Session {
    fn drop(&mut self) {unsafe{
        for &buffer in &self.buffers{if !buffer.is_null(){(self.api.clReleaseMemObject)(buffer);}}
        if !self.kernel.is_null(){(self.api.clReleaseKernel)(self.kernel);}
        if !self.program.is_null(){(self.api.clReleaseProgram)(self.program);}
        if !self.queue.is_null(){(self.api.clReleaseCommandQueue)(self.queue);}
        if !self.context.is_null(){(self.api.clReleaseContext)(self.context);}
    }}
}
fn require_handle(value:Raw,kind:&str)->Result<()>{if value.is_null(){Err(fail(format!("null OpenCL {kind}")))}else{Ok(())}}
fn device_name(api:&Api,device:Raw)->Result<String>{
    let mut size=0;check(unsafe{(api.clGetDeviceInfo)(device,0x102b,0,null_mut(),&mut size)},"device name size")?;
    if size==0||size>65536{return Err(fail("invalid device name size"));}
    let mut bytes=vec![0;size];check(unsafe{(api.clGetDeviceInfo)(device,0x102b,size,bytes.as_mut_ptr().cast(),null_mut())},"device name")?;
    if bytes.last()==Some(&0){bytes.pop();}Ok(String::from_utf8_lossy(&bytes).into_owned())
}
