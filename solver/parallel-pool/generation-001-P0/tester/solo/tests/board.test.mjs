import test from 'node:test';
import assert from 'node:assert/strict';
import {layout,renderBoard,drawDayRoutes,residenceHeatmap,boardPlayerColor,brandColor,ROAD_COLOR} from '../../../common/web/board.js';

test('the default board palette distinguishes all sixteen supported teams',() => {
  const colors = Array.from({length:16},(_,id) => boardPlayerColor(id));
  assert.equal(new Set(colors).size,16);
  assert.equal(boardPlayerColor(16),colors[0]);
});

// Expected measurements come from match #951's public SVG (radius 20, padding 6).
test('map centers and padding match the arena SVG at its native viewBox size',() => {
  const L = layout({map:{width:16,height:16}},583.5767664977294,502);
  assert.equal(L.s,20);
  assert.deepEqual(L.center(0,0),[40.64101615137754,26]);
  assert.deepEqual(L.center(1,0),[23.32050807568877,56]);
  assert.deepEqual(L.center(15,10),[369.7306695894642,476]);
  const half = layout({map:{width:16,height:16}},583.5767664977294 / 2,251);
  assert.equal(half.s,10);
  assert.deepEqual(half.center(1,0),[23.32050807568877 / 2,28]);
});

test('store marks, stock dots and vehicle sizes match the reference SVG, with traffic outlines',() => {
  const commands = [], ctx = {lineWidth:0,globalAlpha:1};
  for (const op of ['beginPath','closePath','moveTo','lineTo','quadraticCurveTo','arc','roundRect','fillRect','translate','save','restore','fillText']) {
    ctx[op] = (...args) => commands.push({op,args,fill:ctx.fillStyle,alpha:ctx.globalAlpha});
  }
  ctx.fill = () => commands.push({op:'fill',color:ctx.fillStyle,alpha:ctx.globalAlpha});
  ctx.stroke = () => commands.push({op:'stroke',color:ctx.strokeStyle,width:ctx.lineWidth});
  const problem = {map:{width:2,height:2,cells:[[0,1],[2,3]]},fuelLimits:20,spots:[{pos:0,brand:8,stocks:2}]};
  const frame = {agents:[{pos:0,kind:0,fuel:10},{pos:1,kind:1,fuel:0}],stocks:[1],pending:[],events:[]};
  const day = {day:1,frames:[frame]}, width = Math.sqrt(3) * 20 * 2.5 + 12;
  const canvas = {width:width + 8,height:90,getContext:() => ctx};
  renderBoard(canvas,problem,{frame,day,roadStatus:new Map([[1,1]])},{showRoute:false,agentColor:() => boardPlayerColor(0)});
  const center = [40.64101615137754,26];
  const stocks = commands.filter(c => c.op === 'arc' && c.args[2] === 1.7);
  assert.deepEqual(stocks.map(c => c.args.slice(0,3)),[[center[0] - 2.3,39.2,1.7],[center[0] + 2.3,39.2,1.7]]);
  assert.ok(commands.some(c => c.op === 'fill' && c.color === '#f08cae'));
  assert.ok(commands.some(c => c.op === 'fill' && c.color === '#ffd98a'));
  assert.ok(commands.some(c => c.op === 'fill' && c.color === 'rgba(255,255,255,0.2)'));
  assert.ok(commands.some(c => c.op === 'arc' && c.args[2] === 6.3 && c.args[0] === center[0]));
  assert.ok(commands.some(c => c.op === 'roundRect' && c.args[2] === 11.6 && c.args[4] === 2.6));
  assert.ok(commands.some(c => c.op === 'roundRect' && c.args[2] === 16 && c.args[3] === 2.6 && c.args[1] === 34.5));
  assert.ok(commands.some(c => c.op === 'fill' && c.color === '#eabf7a')); // Exactly 50% fuel.
  assert.ok(commands.some(c => c.op === 'fill' && c.color === ROAD_COLOR[0]));
  assert.ok(!commands.some(c => c.op === 'fill' && c.color === ROAD_COLOR[1]));
  assert.ok(commands.some(c => c.op === 'stroke' && c.color === ROAD_COLOR[1] && c.width === 2.4));
  assert.ok(commands.some(c => c.op === 'stroke' && c.color === 'rgba(7,11,22,0.9)' && c.width === 1.1));
  assert.deepEqual(commands.filter(c => c.op === 'fillText').map(c => c.args[0]),['0','8']); // Preserve the icon and identify its brand beside it.
  assert.equal(brandColor(18),'#f08cae');
});

test('shop numbers distinguish repeated brand colors at every scale, including depleted and occupied shops',() => {
  for (const scale of [.5,1,2]) {
    const labels = [], badges = [], ctx = {};
    for (const op of ['beginPath','closePath','moveTo','lineTo','quadraticCurveTo','arc','fillRect','translate','save','restore','stroke','fill']) ctx[op] = () => {};
    ctx.roundRect = (...args) => badges.push(args);
    ctx.fillText = (text,x,y,maxWidth) => labels.push({text,x,y,maxWidth,color:ctx.fillStyle,font:ctx.font,alpha:ctx.globalAlpha});
    const problem = {map:{width:4,height:1,cells:[[0,0,0,0]]},fuelLimits:20,
      spots:[0,10,20,27].map((brand,pos) => ({pos,brand,stocks:2}))};
    const frame = {agents:[{pos:2,kind:0,fuel:20}],stocks:[2,1,0,2],pending:[],events:[]}, day = {day:1,frames:[frame]};
    const canvas = {width:(Math.sqrt(3) * 20 * 4.5 + 12) * scale + 8,height:52 * scale + 8,getContext:() => ctx};
    renderBoard(canvas,problem,{frame,day,roadStatus:new Map()},{showIds:true,agentColor:() => boardPlayerColor(0),
      overlay:() => labels.push({text:'opponent overlay'})});
    assert.equal(brandColor(0),brandColor(10));
    assert.equal(brandColor(10),brandColor(20));
    assert.deepEqual(labels.slice(-4).map(label => label.text),['0','10','20','27']);
    const L = layout(problem,canvas.width - 8,canvas.height - 8);
    labels.slice(-4).forEach((label,pos) => {
      const [cx,cy] = L.center(0,pos);
      assert.equal(label.color,'#f8fafc');
      assert.equal(label.alpha,1); // Depleted shops keep their identifiers legible.
      assert.ok(Math.abs(label.x - (cx - 11 * scale)) < 1e-9);
      assert.ok(Math.abs(label.y - (cy + 6.5 * scale * .05)) < 1e-9);
      assert.equal(label.maxWidth,8 * scale);
      assert.ok(label.font.startsWith(`800 ${6.5 * scale}px `));
    });
    assert.deepEqual(badges.slice(-4).map(args => args.slice(2)),Array(4).fill([10 * scale,9 * scale,2 * scale]));
  }
});

test('traffic outlines appear in the normal view and are absent from the heatmap',() => {
  for (const mode of ['none','both']) {
    const fills = [], strokes = [], ctx = {};
    for (const op of ['beginPath','closePath','moveTo','lineTo','fillRect','translate','save','restore']) ctx[op] = () => {};
    ctx.fill = () => fills.push(ctx.fillStyle);
    ctx.stroke = () => strokes.push([ctx.strokeStyle,ctx.lineWidth]);
    const problem = {map:{width:3,height:1,cells:[[1,1,1]]},spots:[]}, frame = {step:0,agents:[{kind:0,pos:0}],stocks:[],events:[]};
    const day = {day:1,frames:[frame,{step:1,agents:[{kind:0,pos:1}]}]};
    const canvas = {width:Math.sqrt(3) * 20 * 3.5 + 20,height:60,getContext:() => ctx};
    // No vehicles in normal mode; its drawing is covered in the preceding test.
    const view = mode === 'none' ? {frame:{...frame,agents:[]},day:{day:1,frames:[{...frame,agents:[]}]}} : {frame,day};
    renderBoard(canvas,problem,{...view,roadStatus:new Map([[1,1],[2,2]])},{showRoute:false,heatmapMode:mode,selectedCell:mode === 'both' ? 1 : null});
    assert.deepEqual(fills,mode === 'none' ? Array(3).fill(ROAD_COLOR[0]) : ['rgb(37,99,235)','#f1f5f9','#f1f5f9']);
    const rim = ['rgba(7,11,22,0.9)',1.1];
    assert.deepEqual(strokes,mode === 'both' ? [] : [rim,rim,[ROAD_COLOR[1],2.4],rim,[ROAD_COLOR[2],2.4]]);
  }
});

test('road outlines stay concentric and fit inside all six cell edges at every render scale',() => {
  for (const scale of [.5,1,2,3]) {
    const fills = [], strokes = [], ctx = {};
    let path;
    ctx.beginPath = () => {path = [];};
    ctx.moveTo = ctx.lineTo = (x,y) => path.push([x,y]);
    for (const op of ['closePath','fillRect','translate','save','restore']) ctx[op] = () => {};
    ctx.fill = () => fills.push(path.slice());
    ctx.stroke = () => strokes.push({path:path.slice(),color:ctx.strokeStyle,width:ctx.lineWidth,join:ctx.lineJoin});
    const problem = {map:{width:3,height:3,cells:[[0,0,0],[0,1,0],[0,0,0]]},spots:[]};
    const frame = {agents:[],stocks:[],events:[]}, day = {day:1,frames:[frame]};
    const canvas = {width:(Math.sqrt(3) * 20 * 3.5 + 12) * scale + 8,height:112 * scale + 8,getContext:() => ctx};
    renderBoard(canvas,problem,{frame,day,roadStatus:new Map([[4,1]])},{heatmapMode:'none'});
    const cell = fills[4], outline = strokes.find(s => s.color === ROAD_COLOR[1]);
    const [cx,cy] = layout(problem,canvas.width - 8,canvas.height - 8).center(1,1);
    assert.equal(outline.path.length,6);
    assert.equal(outline.join,'miter');
    assert.ok(Math.abs(outline.width - 2.4 * scale) < 1e-9);
    for (let i = 0; i < 6; i++) {
      const a = outline.path[i], b = outline.path[(i + 1) % 6], opposite = outline.path[(i + 3) % 6];
      assert.ok(Math.abs((a[0] + opposite[0]) / 2 - cx) < 1e-9);
      assert.ok(Math.abs((a[1] + opposite[1]) / 2 - cy) < 1e-9);
      const edge = cell[i], distance = Math.abs((b[0] - a[0]) * (edge[1] - a[1]) - (b[1] - a[1]) * (edge[0] - a[0])) / Math.hypot(b[0] - a[0],b[1] - a[1]);
      // Half of the centered stroke fits between its centerline and the cell boundary.
      // The reference SVG rounds hex vertices to 0.01, so allow that tiny geometric error.
      assert.ok(Math.abs(distance - outline.width / 2) < .001 * scale);
      const corner = Math.hypot(a[0] - cx,a[1] - cy) + outline.width / Math.sqrt(3);
      assert.ok(corner <= Math.hypot(edge[0] - cx,edge[1] - cy) + .001 * scale);
    }
  }
});

test('daily patrol and supply routes match the arena dash, opacity and vehicle hue specifications at every scale',() => {
  const day = {frames:[
    {agents:[{kind:0,pos:0},{kind:1,pos:3}]},
    {agents:[{kind:0,pos:0},{kind:1,pos:4}]},
    {agents:[{kind:0,pos:1},{kind:1,pos:4}]},
    {agents:[{kind:0,pos:2},{kind:1,pos:3}]},
  ]};
  for (const size of [10,20,40]) {
    const strokes = [], state = {lineWidth:7,globalAlpha:.4,lineCap:'butt',lineJoin:'miter',dash:[9]}, ctx = {...state};
    let saved, points;
    ctx.save = () => {saved = Object.fromEntries(Object.keys(state).map(key => [key,ctx[key]]));};
    ctx.restore = () => Object.assign(ctx,saved);
    ctx.setLineDash = dash => {ctx.dash = dash;};
    ctx.beginPath = () => {points = [];};
    ctx.moveTo = ctx.lineTo = (x,y) => points.push([x,y]);
    ctx.stroke = () => strokes.push({points,width:ctx.lineWidth,dash:ctx.dash,alpha:ctx.globalAlpha,cap:ctx.lineCap,color:ctx.strokeStyle});
    const center = pos => [pos * 10,0], k = size / 20;
    const draw = d => drawDayRoutes(ctx,center,size,d,() => boardPlayerColor(0));
    draw(day);
    assert.deepEqual(strokes,[
      {points:[[0,0],[10,0],[20,0]],width:1.4 * k,dash:[3 * k,4 * k],alpha:.65,cap:'round',color:'hsl(184.9, 85%, 67%)'},
      {points:[[30,0],[40,0],[30,0]],width:1.4 * k,dash:[3 * k,4 * k],alpha:.65,cap:'round',color:'hsl(230.9, 85%, 67%)'},
    ]);
    assert.deepEqual(Object.fromEntries(Object.keys(state).map(key => [key,ctx[key]])),state);
    draw(day);
    assert.deepEqual(strokes.slice(2),strokes.slice(0,2)); // Daily path stays fixed during replay.
    draw({frames:[day.frames[0],{agents:[{kind:0,pos:3},{kind:1,pos:3}]}]});
    assert.equal(strokes.length,5); // A stationary supply vehicle has no route.
    assert.deepEqual(strokes[4].points,[[0,0],[30,0]]); // A new day has its own path.
  }
});

function residenceDay(day = 1) {
  return {day,frames:[
    {step:0,agents:[{kind:0,pos:0},{kind:1,pos:0}],pending:[{destination:1,until:2},null]},
    {step:2,agents:[{kind:0,pos:1},{kind:1,pos:0}]},
    {step:5,agents:[{kind:0,pos:0},{kind:1,pos:1}]},
  ]};
}

test('combined residence counts sum both vehicle types per cell across the selected days and teams',() => {
  const day = residenceDay(), nextDay = {day:2,frames:[
    {step:0,agents:[{kind:0,pos:2},{kind:1,pos:2}]},
    {step:4,agents:[{kind:0,pos:0},{kind:1,pos:1}]},
  ]};
  const opponentDay = {day:1,frames:[
    {step:0,agents:[{kind:0,pos:0},{kind:1,pos:1}]},
    {step:6,agents:[{kind:0,pos:2},{kind:1,pos:2}]},
  ]};
  const matchReplay = {days:[day,nextDay],opponents:[{days:[opponentDay]}]}, options = {matchReplay,player:0,heatmapScope:'day',heatmapTeams:'selected'};
  const counts = residenceHeatmap(day,3,options);
  assert.deepEqual(counts.patrol,[2,3,0]); // Count movement at its departure cell; exclude the final frame.
  assert.deepEqual(counts.supply,[5,0,0]);
  assert.deepEqual(counts.total,[7,3,0]);
  assert.equal(counts.maxTotal,7); // The maximum total is per cell, not maxPatrol + maxSupply (8).
  assert.equal(residenceHeatmap(day,3,options),counts);
  assert.deepEqual(residenceHeatmap(nextDay,3,options).total,[0,0,8]);
  assert.deepEqual(residenceHeatmap(day,3,{...options,heatmapScope:'all'}).total,[7,3,8]);
  assert.deepEqual(residenceHeatmap(day,3,{...options,heatmapTeams:'all'}).total,[13,9,0]);
  assert.deepEqual(residenceHeatmap(day,3,{...options,heatmapScope:'all',heatmapTeams:'all'}).total,[13,9,8]);
  assert.deepEqual(residenceHeatmap(day,3,{...options,player:1}).total,[6,6,0]);
});

test('heatmaps fill each whole cell without numbers or outlines in combined and individual vehicle modes',() => {
  for (const mode of ['both','patrol','supply']) {
    const fills = [], labels = [], rectangles = [], strokes = [], ctx = {};
    for (const op of ['beginPath','closePath','moveTo','lineTo','quadraticCurveTo','translate','save','restore']) ctx[op] = () => {};
    ctx.stroke = () => strokes.push([ctx.strokeStyle,ctx.lineWidth]);
    ctx.fill = () => fills.push(ctx.fillStyle);
    ctx.fillText = (...args) => labels.push(args);
    ctx.fillRect = (...args) => rectangles.push(args);
    const day = residenceDay(), frame = day.frames[0];
    const problem = {map:{width:2,height:2,cells:[[0,1],[2,3]]},spots:[{pos:0,brand:0,stocks:1}]};
    const canvas = {width:Math.sqrt(3) * 20 * 2.5 + 20,height:90,getContext:() => ctx};
    renderBoard(canvas,problem,{frame,day,roadStatus:new Map()},{heatmapMode:mode,showIds:true,showRoute:true,selectedCell:0});
    assert.deepEqual(labels,[]);
    assert.deepEqual(strokes,[['rgba(120,180,255,0.4)',1.3]]); // Pond waves only; no cell, traffic or selection outline.
    assert.equal(rectangles.length,1); // Only the board background; no upper/lower half-cell fills.
    assert.equal(fills.length,4);
    assert.equal(fills[2],'#f1f5f9'); // Unvisited walkable cell.
    assert.equal(fills[3],'#cbd5e1'); // Impassable pond.
    if (mode === 'both') assert.deepEqual(fills.slice(0,2),['rgb(37,99,235)','rgb(104,148,242)']);
    if (mode === 'patrol') assert.equal(fills[1],'rgb(37,99,235)');
    if (mode === 'supply') assert.deepEqual(fills.slice(0,2),['rgb(234,88,12)','#f1f5f9']);
  }
});

test('logarithmic colors reveal low residence counts and remain finite for unvisited cells',() => {
  for (const kind of [0,1]) {
    const fills = [], ctx = {};
    for (const op of ['beginPath','closePath','moveTo','lineTo','fillRect','translate','save','restore','stroke']) ctx[op] = () => {};
    ctx.fill = () => fills.push(ctx.fillStyle);
    const day = {day:1,frames:[0,1,10,109,1108].map((step,pos) => ({step,agents:[{kind,pos}]}))};
    const problem = {map:{width:5,height:1,cells:[[0,0,0,0,0]]},spots:[]};
    const canvas = {width:Math.sqrt(3) * 20 * 5.5 + 20,height:60,getContext:() => ctx};
    const render = (day,mode) => renderBoard(canvas,problem,{frame:day.frames[0],day,roadStatus:new Map()},{heatmapMode:mode});
    render(day,kind === 0 ? 'patrol' : 'supply');
    // Counts 1, 9, 99, 999 become log(2)/log(1000), 1/3, 2/3, 1.
    const expected = kind === 0
      ? ['rgb(219,231,253)','rgb(172,197,248)','rgb(104,148,242)','rgb(37,99,235)','#f1f5f9']
      : ['rgb(253,231,214)','rgb(248,194,162)','rgb(241,141,87)','rgb(234,88,12)','#f1f5f9'];
    assert.deepEqual(fills,expected);
    fills.length = 0;
    render({day:2,frames:[day.frames[0]]},'both');
    assert.deepEqual(fills,Array(5).fill('#f1f5f9')); // No elapsed intervals, so max = 0.
  }
});
