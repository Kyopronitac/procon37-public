use procon37_distributed_gpu_lab::*;
use std::sync::Arc;

fn map(width:usize,height:usize,mut seed:u64)->Prepared {
    let n=width*height;
    let mut p=Prepared{neighbors:vec![[NO_CELL;6];n],passable:vec![1;n],durations:vec![2;n],costs:vec![1;n],shops:vec![0;n],targets:vec![]};
    for y in 0..height {for x in 0..width {
        let cell=y*width+x;
        let left=if y%2==0{0}else{-1};
        for (d,(dy,dx)) in [(-1,left),(-1,left+1),(0,1),(1,left+1),(1,left),(0,-1)].into_iter().enumerate(){
            let yy=y as isize+dy;let xx=x as isize+dx;
            if yy>=0&&xx>=0&&yy<height as isize&&xx<width as isize{p.neighbors[cell][d]=(yy as usize*width+xx as usize)as u16;}
        }
        seed^=seed<<13;seed^=seed>>7;seed^=seed<<17;
        p.durations[cell]=1+(seed%4)as i32;
        p.costs[cell]=1+((seed>>5)%2)as i32;
        p.shops[cell]=u8::from((seed>>8)%5==0);
        p.passable[cell]=u8::from(cell==0||(seed>>12)%9!=0);
    }}
    let passable=(0..n).filter(|&v|p.passable[v]!=0).collect::<Vec<_>>();
    p.targets=vec![0,passable[passable.len()/2]as u16,*passable.last().unwrap()as u16,0];
    p
}

#[test]
fn synchronous_relaxation_matches_strict_heap_and_tree_scalars(){
    let mut entries=0;
    for seed in 1..=24 {
        let p=Arc::new(map(3+seed as usize%7,3+(seed as usize*3)%6,seed));
        let heap=cpu_reference(&p).unwrap();
        let parallel=jacobi_reference(&p).unwrap();
        assert_eq!(parallel,heap,"seed={seed}");
        let checked=parallel.verify(p.clone()).unwrap();
        let scalars=checked.scalars().unwrap();
        let n=p.cells();
        for mode in 0..ROUTE_MODES {for (ti,&target) in p.targets.iter().enumerate(){
            let base=(mode*p.targets.len()+ti)*n;
            for source in 0..n {
                if heap.distances[base+source]==INF{continue;}
                let mut cell=source;let mut time=0;let mut fuel=0;let mut path=vec![];
                while cell!=usize::from(target){
                    time+=p.durations[cell];fuel+=p.costs[cell];
                    cell=usize::from(heap.next[base+cell]);path.push(cell);
                    assert!(path.len()<n);
                }
                let mut fingerprint=0u64;
                for &cell in path.iter().rev(){fingerprint=(fingerprint^(cell as u64).wrapping_add(0x9e3779b185ebca87)).rotate_left(17).wrapping_mul(0xc2b2ae3d27d4eb4f);}
                assert_eq!((scalars.times[base+source],scalars.fuels[base+source],usize::from(scalars.lengths[base+source]),scalars.fingerprints[base+source]),(time,fuel,path.len(),fingerprint));
                entries+=1;
            }
        }}
    }
    assert!(entries>10_000);
    eprintln!("CPU-only strict heap/Jacobi/scalar checked routes={entries}");
}

#[test]
fn exact_tie_chooses_largest_cell_not_neighbor_direction(){
    let mut p=Prepared{neighbors:vec![[NO_CELL;6];4],passable:vec![1;4],durations:vec![2;4],costs:vec![1;4],shops:vec![0;4],targets:vec![3,3]};
    p.neighbors[0][0]=1;p.neighbors[0][1]=2;
    p.neighbors[1][0]=0;p.neighbors[1][1]=3;
    p.neighbors[2][0]=0;p.neighbors[2][1]=3;
    p.neighbors[3][0]=1;p.neighbors[3][1]=2;
    let p=Arc::new(p);let output=jacobi_reference(&p).unwrap();
    assert_eq!(output,cpu_reference(&p).unwrap());
    assert_eq!(output.next[0],2);
    let mut wrong_tie=output.clone();wrong_tie.next[0]=1;
    assert!(wrong_tie.verify(p.clone()).unwrap_err().0.contains("deterministic next"));
    output.verify(p).unwrap();
}

#[test]
fn fuel_minimum_is_separate_from_time_route_fuel(){
    let mut p=Prepared{neighbors:vec![[NO_CELL;6];4],passable:vec![1;4],durations:vec![1,1,4,1],costs:vec![1,2,1,1],shops:vec![0;4],targets:vec![3]};
    p.neighbors[0][0]=1;p.neighbors[0][1]=2;
    p.neighbors[1][0]=0;p.neighbors[1][1]=3;
    p.neighbors[2][0]=0;p.neighbors[2][1]=3;
    p.neighbors[3][0]=1;p.neighbors[3][1]=2;
    let p=Arc::new(p);let checked=cpu_reference(&p).unwrap().verify(p.clone()).unwrap();
    assert_eq!(checked.raw().next[0],1);
    assert_eq!(checked.scalars().unwrap().fuels[0],3);
    assert_eq!(checked.raw().distances[ROUTE_MODES*p.entries()],2);
}

#[test]
fn rejects_bad_shapes_weights_asymmetry_and_corrupt_gpu_tables(){
    let p=map(5,5,17);
    let mut wrong=p.clone();wrong.durations[0]=0;assert!(wrong.validate().is_err());
    let mut wrong=p.clone();wrong.targets[0]=NO_CELL;assert!(wrong.validate().is_err());
    let mut wrong=p.clone();wrong.neighbors[0][0]=3;assert!(wrong.validate().is_err());
    let mut wrong=p.clone();wrong.costs.pop();assert!(wrong.validate().is_err());
    let mut output=cpu_reference(&p).unwrap();output.distances[0]=INF;assert!(output.verify(Arc::new(p)).is_err());
}

#[test]
fn one_cell_and_disconnected_components_keep_zero_unused_next(){
    let mut p=map(1,1,2);p.targets=vec![0,0];
    let output=jacobi_reference(&p).unwrap();
    assert!(output.distances.iter().all(|&v|v==0));assert!(output.next.iter().all(|&v|v==0));
    output.verify(Arc::new(p)).unwrap();
    let p=Prepared{neighbors:vec![[NO_CELL;6];3],passable:vec![1,0,1],durations:vec![2;3],costs:vec![1;3],shops:vec![0;3],targets:vec![0,2]};
    let output=jacobi_reference(&p).unwrap();assert_eq!(output,cpu_reference(&p).unwrap());
    output.verify(Arc::new(p)).unwrap().scalars().unwrap();
}

#[test]
fn maximum_32_square_shape_and_duplicate_targets(){
    let p=Arc::new(map(32,32,99));
    assert_eq!(p.cells(),MAX_CELLS);
    let output=jacobi_reference(&p).unwrap();assert_eq!(output,cpu_reference(&p).unwrap());
    let result=output.verify(p).unwrap();result.scalars().unwrap();
}
