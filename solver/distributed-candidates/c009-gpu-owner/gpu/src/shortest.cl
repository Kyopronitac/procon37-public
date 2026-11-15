// Integer-only reverse distances. One workgroup owns one (mode,target).
// The route successor is chosen AFTER convergence, independent of lane order.
#define INF 2147483647
#define NO_CELL 65535

int departure_weight(uint mode, uint cell,
                     __global const int *durations, __global const int *costs,
                     __global const uchar *shops) {
    if (mode == 6) return costs[cell];
    const int A[6] = {100,100,10,100,100,100};
    const int B[6] = {3,100,100,3,3,3};
    const int R[6] = {0,0,0,20,80,160};
    return max(1, durations[cell]*A[mode] + costs[cell]*B[mode]
                   - (shops[cell] ? R[mode] : 0));
}

__kernel void build_tables(uint n, uint target_count,
                          __global const ushort *neighbors,
                          __global const uchar *passable,
                          __global const int *durations,
                          __global const int *costs,
                          __global const uchar *shops,
                          __global const ushort *targets,
                          __global int *distances,
                          __global ushort *next,
                          __global uint *rounds,
                          __local int *first, __local int *second,
                          volatile __local int *changed) {
    uint job=get_group_id(0), mode=job/target_count, ti=job%target_count;
    uint target=targets[ti], lane=get_local_id(0), width=get_local_size(0);
    uint base=job*n;
    for(uint y=lane;y<n;y+=width) first[y]=(y==target?0:INF);
    barrier(CLK_LOCAL_MEM_FENCE);
    __local int *before=first, *after=second;
    uint count=0, converged=0;
    for(uint round=0;round<n;++round) {
        if(lane==0) *changed=0;
        barrier(CLK_LOCAL_MEM_FENCE);
        for(uint y=lane;y<n;y+=width) {
            int best=before[y];
            if(passable[y] && y!=target) {
                int weight=departure_weight(mode,y,durations,costs,shops);
                for(uint dir=0;dir<6;++dir) {
                    uint x=neighbors[y*6+dir];
                    if(x!=NO_CELL && passable[x] && before[x]!=INF)
                        best=min(best,before[x]+weight);
                }
            }
            after[y]=best;
            if(best!=before[y]) atomic_or(changed,1);
        }
        barrier(CLK_LOCAL_MEM_FENCE);
        __local int *temporary=before;before=after;after=temporary;
        count=round+1;
        if(*changed==0) {converged=1;break;}
        // No lane may overwrite the next round's flag until all read it.
        barrier(CLK_LOCAL_MEM_FENCE);
    }
    for(uint y=lane;y<n;y+=width) {
        distances[base+y]=before[y];
        if(mode<6) {
            ushort successor=0;
            if(passable[y] && y!=target && before[y]!=INF) {
                int weight=departure_weight(mode,y,durations,costs,shops);
                for(uint dir=0;dir<6;++dir) {
                    uint x=neighbors[y*6+dir];
                    if(x!=NO_CELL && passable[x] && before[x]!=INF && before[x]+weight==before[y])
                        successor=max(successor,(ushort)x);
                }
            }
            next[base+y]=successor;
        }
    }
    if(lane==0) rounds[job]=count | (converged?0:0x80000000u);
}
