use std::cmp::Reverse;
use std::collections::BinaryHeap;
use tester::model::*;

#[derive(Clone, Default)]
pub struct Path {
    pub dirs: Vec<i64>,
    pub cells: Vec<usize>,
    pub time: u16,
    pub fuel: u16,
}

pub struct Network {
    pub nodes: Vec<usize>,
    pub paths: Vec<Path>,
    pub duration: Vec<u16>,
    pub cost: Vec<u16>,
    pub spot: Vec<i8>,
    pub brands: Vec<u32>,
    pub density: Vec<f64>,
    pub exit_need: Vec<u16>,
    pub n: usize,
    pub variants: usize,
    node_at: Vec<usize>,
    brand_chunks: [[u32;256];4],
}

impl Network {
    // Only the four-car daily planner adds a fuel-minimal route. Selection
    // uses the original three route variants to keep its forecasts stable.
    pub fn new(p: &Problem, agents: &[Agent], traffic: &[Traffic]) -> Self {
        Self::with_variants(p,agents,traffic,if p.agents.len()==4{4}else{3})
    }
    pub fn with_variants(p: &Problem, agents: &[Agent], traffic: &[Traffic], variants:usize) -> Self {
        let m = &p.map;
        let mut status = vec![0; m.len()];
        for x in traffic { status[x.pos] = x.status as usize; }
        let duration: Vec<u16> = (0..m.len()).map(|i| match m.terrain(i) { 0=>2, 1=>[1,2,4][status[i]], 2=>3, _=>999 }).collect();
        let cost: Vec<u16> = (0..m.len()).map(|i| if m.terrain(i)==0 {1} else {2}).collect();
        let mut nodes: Vec<usize> = p.spots.iter().map(|s|s.pos).collect();
        for a in agents { if !nodes.contains(&a.pos) { nodes.push(a.pos); } }
        // Rendezvous need not happen inside a shop. Keep accessible junctions
        // spread across the board so both vehicle types can meet on their way.
        for gy in 0..3 {for gx in 0..3 {
            let cy=(2*gy+1)*m.height/6;let cx=(2*gx+1)*m.width/6;
            let mut best=None;
            for y in cy.saturating_sub(2)..=(cy+2).min(m.height-1) {for x in cx.saturating_sub(2)..=(cx+2).min(m.width-1) {
                let pos=y*m.width+x;if m.terrain(pos)==3||nodes.contains(&pos){continue}
                let links=m.neighbors(pos).filter(|&q|m.terrain(q)!=3).count() as i32;
                let score=links*4-(x.abs_diff(cx)+y.abs_diff(cy)) as i32*3+if m.terrain(pos)==0{4}else if m.terrain(pos)==1{2}else{0};
                if best.is_none_or(|(v,_)|score>v){best=Some((score,pos));}
            }}
            if let Some((_,pos))=best {nodes.push(pos);}
        }}
        let n = nodes.len();
        let mut spot = vec![-1; m.len()];
        let mut brand_ids = Vec::new();
        let brands: Vec<u32> = p.spots.iter().enumerate().map(|(i,s)|{
            spot[s.pos] = i as i8;
            let b = if let Some(b)=brand_ids.iter().position(|&x|x==s.brand) {b} else {brand_ids.push(s.brand);brand_ids.len()-1};
            1u32<<b
        }).collect();
        let mut node_at=Vec::new();let mut brand_chunks=[[0u32;256];4];
        if p.agents.len()<=5 {
            node_at=vec![usize::MAX;m.len()];
            for (node,&pos) in nodes.iter().enumerate(){if node_at[pos]==usize::MAX{node_at[pos]=node;}}
            for chunk in 0..4{for mask in 1usize..256{
                let bit=mask.trailing_zeros() as usize+chunk*8;
                brand_chunks[chunk][mask]=brand_chunks[chunk][mask&(mask-1)]|brands.get(bit).copied().unwrap_or(0);
            }}
        }
        // Any simple path takes at most map.len()*4 steps, so the final
        // weight minimizes fuel first and travel time second.
        let mut paths = vec![Path::default(); variants*n*n];
        for (a,&src) in nodes.iter().enumerate() {
            for (v, fuel_weight) in [0u32, 5, 22, m.len() as u32*40+1].into_iter().take(variants).enumerate() {
                let prev=shortest_tree(p,&duration,&cost,src,fuel_weight);
                for (b,&dst) in nodes.iter().enumerate() {
                    let mut cells=vec![dst]; let mut dirs=Vec::new(); let mut at=dst;
                    while at!=src {
                        let (pr,dir)=prev[at]; if pr==usize::MAX { break }
                        dirs.push(dir); cells.push(pr); at=pr;
                    }
                    dirs.reverse();cells.reverse();
                    let time=cells.iter().take(dirs.len()).map(|&x|duration[x]).sum();
                    let fuel=cells.iter().take(dirs.len()).map(|&x|cost[x]).sum();
                    paths[(a*n+b)*variants+v]=Path {dirs,cells,time,fuel};
                }
            }
        }
        let density=(0..n).map(|a|(0..p.spots.len()).map(|b|85.0*(-(paths[(a*n+b)*variants].time as f64)/13.0).exp()).sum()).collect();
        let exit_need=(0..n).map(|a|{
            let mut fuel:Vec<u16>=(0..p.spots.len()).filter(|&b|nodes[a]!=nodes[b]).map(|b|paths[(a*n+b)*variants+2].fuel).collect();fuel.sort_unstable();
            (fuel.get(1).copied().unwrap_or(8)*3/2+4).min(p.fuel_limits as u16)
        }).collect();
        Self {nodes,paths,duration,cost,spot,brands,density,exit_need,n,variants,node_at,brand_chunks}
    }
    pub fn route_anchors(p:&Problem,traffic:&[Traffic],routes:&[(usize,&[i64])])->Vec<usize> {
        let mut status=vec![0;p.map.len()];for tr in traffic{status[tr.pos]=tr.status as usize;}
        let duration:Vec<u16>=(0..p.map.len()).map(|i|match p.map.terrain(i){0=>2,1=>[1,2,4][status[i]],2=>3,_=>999}).collect();
        let cost:Vec<u16>=(0..p.map.len()).map(|i|if p.map.terrain(i)==0{1}else{2}).collect();
        let variants=if p.agents.len()==4{4}else{3};let all_variants=(1u8<<variants)-1;
        let mut trees=std::collections::HashMap::<usize,[Vec<(usize,i64)>;4]>::new();let mut anchors=Vec::new();
        for &(start,actions) in routes {
            let mut pos=start;let mut anchor=start;let mut active=all_variants;
            for &action in actions {
                if action<0{if !anchors.contains(&pos){anchors.push(pos);}anchor=pos;active=all_variants;continue}
                let dest=p.map.neighbor(pos,action as usize).unwrap();
                loop {
                    let prev=trees.entry(anchor).or_insert_with(||std::array::from_fn(|v|if v<variants{shortest_tree(p,&duration,&cost,anchor,[0u32,5,22,p.map.len() as u32*40+1][v])}else{Vec::new()}));
                    let mut possible=0u8;for v in 0..variants{if active&(1<<v)!=0&&prev[v][dest]==(pos,action){possible|=1<<v;}}
                    if possible!=0{active=possible;break}
                    assert!(anchor!=pos,"adjacent path must be represented");
                    if !anchors.contains(&pos){anchors.push(pos);}anchor=pos;active=all_variants;
                }
                pos=dest;
                if p.spots.iter().any(|s|s.pos==pos){if !anchors.contains(&pos){anchors.push(pos);}anchor=pos;active=all_variants;}
            }
            if !anchors.contains(&pos){anchors.push(pos);}
        }
        anchors
    }
    pub fn path(&self,a:usize,b:usize,v:usize)->&Path {&self.paths[(a*self.n+b)*self.variants+v]}
    pub fn node(&self,pos:usize)->usize {self.find_node(pos).unwrap()}
    pub fn find_node(&self,pos:usize)->Option<usize> {if self.node_at.is_empty(){return self.nodes.iter().position(|&x|x==pos)}let n=self.node_at[pos];if n==usize::MAX{None}else{Some(n)}}
    pub fn brand_mask(&self,spots:u32)->u32 {
        if self.node_at.is_empty(){let mut mask=0;let mut bits=spots;while bits!=0{let i=bits.trailing_zeros() as usize;bits&=bits-1;mask|=self.brands[i];}return mask}
        self.brand_chunks[0][(spots&255) as usize]|self.brand_chunks[1][((spots>>8)&255) as usize]
            |self.brand_chunks[2][((spots>>16)&255) as usize]|self.brand_chunks[3][(spots>>24) as usize]
    }
}

fn shortest_tree(p:&Problem,duration:&[u16],cost:&[u16],src:usize,fuel_weight:u32)->Vec<(usize,i64)> {
    let m=&p.map;let mut dist=vec![u32::MAX;m.len()];let mut prev=vec![(usize::MAX,0i64);m.len()];
    dist[src]=0;let mut heap=BinaryHeap::from([(Reverse(0u32),src)]);
    while let Some((Reverse(d),x))=heap.pop(){
        if d!=dist[x]{continue}
        for dir in 0..6{if let Some(y)=m.neighbor(x,dir).filter(|&y|m.terrain(y)!=3){
            let nd=d+duration[x] as u32*10+cost[x] as u32*fuel_weight;
            if nd<dist[y]{dist[y]=nd;prev[y]=(x,dir as i64);heap.push((Reverse(nd),y));}
        }}
    }
    prev
}
