// 非公式練習場のHTML/SVGに合わせた盤面描画。単独・対戦・対戦サーバーで共有する。
// 半径20・余白6の参照座標を拡大縮小し、点線経路・滑らかな移動・ヒートマップを重ねる。
export const TERRAIN_COLOR = {0:'#4d3b28', 2:'#4f5c2e', 3:'#164a78'};
export const ROAD_COLOR = ['#5b6572','#e8b23d','#e2503f'];
const PLAYER_COLORS = ['#64b0f2','#f2836b','#7fd6a4','#c9a0f0','#eed268','#6cd9d0','#ef92c0','#a9c069',
  '#9ab8ff','#ffad76','#a1dd79','#eea4e6','#f5b5ce','#85cdda','#f1dd9c','#d1b1a1'];
export const boardPlayerColor = player => PLAYER_COLORS[player % PLAYER_COLORS.length];
export const SUPPLY_COLOR = PLAYER_COLORS[0];
export const AGENT_COLOR = [PLAYER_COLORS[0],SUPPLY_COLOR];
const BRAND_COLORS = ['#f5b83d','#ec6a4c','#57c785','#5f93f5','#cf6ee4','#48c4cf','#e9e15f','#9b7bf7','#f08cae','#8fc74f'];
const RIM = 'rgba(7,11,22,0.9)', SELECT = '#eabf7a', ACQUIRED = '#ffd98a', REFUEL = '#7fd6a4';
export const brandColor = brand => BRAND_COLORS[brand % BRAND_COLORS.length];
const fuelColor = ratio => ratio > .5 ? '#7fd6a4' : ratio > .25 ? '#eabf7a' : '#ef6b5a';
export const BOARD_LEGEND = [['平地',TERRAIN_COLOR[0]],['山地',TERRAIN_COLOR[2]],['池（進入不可）',TERRAIN_COLOR[3]],
  ['道路 / 順調',ROAD_COLOR[0]],['道路の枠 / 混雑',ROAD_COLOR[1]],['道路の枠 / 渋滞',ROAD_COLOR[2]],
  ['店舗（色・番号=系列、うどん記号、点=在庫）',BRAND_COLORS[0]]];

export function layout(config,cw,ch) {
  const H = config.map.height, W = config.map.width;
  const width = Math.sqrt(3) * 20 * (W + .5) + 12, height = (1.5 * H + .5) * 20 + 12;
  const scale = Math.min(cw / width,ch / height), s = 20 * scale, hw = Math.sqrt(3) * s;
  const x = (cw - width * scale) / 2 + 6 * scale, y = (ch - height * scale) / 2 + 6 * scale;
  return {s,hw,H,W,center(r,c) {return [x + c * hw + hw / 2 + (r % 2 === 0 ? hw / 2 : 0),y + s + r * 1.5 * s];}};
}
function hexPath(ctx,cx,cy,s,referenceRadius = s) {
  ctx.beginPath();
  for (let k = 0; k < 6; k++) {
    const a = Math.PI / 180 * (60 * k - 30), scale = s / referenceRadius;
    const x = cx + Number((referenceRadius * Math.cos(a)).toFixed(2)) * scale, y = cy + Number((referenceRadius * Math.sin(a)).toFixed(2)) * scale;
    if (k === 0) ctx.moveTo(x,y); else ctx.lineTo(x,y);
  }
  ctx.closePath();
}
function roundedRect(ctx,x,y,w,h,r) {if (ctx.roundRect) ctx.roundRect(x,y,w,h,r); else ctx.rect(x,y,w,h);}
function circle(ctx,x,y,r,color,width) {ctx.beginPath(); ctx.arc(x,y,r,0,Math.PI * 2); ctx.strokeStyle = color; ctx.lineWidth = width; ctx.stroke();}
function label(ctx,text,x,y,size,color,maxWidth) {
  ctx.fillStyle = color; ctx.font = `800 ${size}px -apple-system, BlinkMacSystemFont, "Hiragino Sans", "Noto Sans JP", "Segoe UI", sans-serif`; ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
  ctx.fillText(String(text),x,y + size * .05,maxWidth);
}
// 巡回車は半径6.3の円、補給車は11.6四方の角丸四角と給油の滴。
export function drawAgent(ctx,x,y,rad,kind,color) {
  const k = rad / 6.3;
  ctx.beginPath();
  if (kind === 0) ctx.arc(x,y,rad,0,Math.PI * 2); else roundedRect(ctx,x - 5.8 * k,y - 5.8 * k,11.6 * k,11.6 * k,2.6 * k);
  ctx.fillStyle = color; ctx.fill(); ctx.strokeStyle = RIM; ctx.lineWidth = 1.2 * k; ctx.stroke();
  if (kind !== 0) {
    ctx.beginPath(); ctx.moveTo(x,y - 3.4 * k);
    ctx.quadraticCurveTo(x + 3 * k,y,x,y + 2.8 * k); ctx.quadraticCurveTo(x - 3 * k,y,x,y - 3.4 * k); ctx.closePath();
    ctx.fillStyle = 'rgba(16,22,42,0.8)'; ctx.fill();
  }
}
export function drawAgentLabel(ctx,x,y,rad,text) {
  const k = rad / 6.3;
  ctx.fillStyle = '#10162a'; ctx.font = `800 ${7 * k}px -apple-system, BlinkMacSystemFont, "Hiragino Sans", "Noto Sans JP", "Segoe UI", sans-serif`;
  ctx.textAlign = 'center'; ctx.textBaseline = 'alphabetic'; ctx.fillText(String(text),x,y + 2.8 * k);
}
export function drawVehicle(ctx,x,y,size,agent,color,id,fuelLimits) {
  const k = size / 20, rad = 6.3 * k;
  drawAgent(ctx,x,y,rad,agent.kind,color);
  if (agent.kind !== 0) return;
  drawAgentLabel(ctx,x,y,rad,id);
  const ratio = Math.max(0,Math.min(1,agent.fuel / fuelLimits));
  ctx.beginPath(); roundedRect(ctx,x - 8 * k,y + 8.5 * k,16 * k,2.6 * k,1.3 * k); ctx.fillStyle = 'rgba(255,255,255,0.15)'; ctx.fill();
  if (ratio > 0) {ctx.beginPath(); roundedRect(ctx,x - 8 * k,y + 8.5 * k,16 * k * ratio,2.6 * k,1.3 * k); ctx.fillStyle = fuelColor(ratio); ctx.fill();}
}
// 参照ページと同じく、表示中の全チームをまとめて同一セルの車の配置を決める。
export function boardAgentOffsets(settings,day,step,size,fallback = []) {
  const groups = new Map(), result = new Map(), player = settings.player ?? 0;
  const players = settings.showOthers && settings.matchReplay ? [settings.matchReplay,...(settings.matchReplay.opponents || [])] : null;
  const append = (agents,id) => agents.forEach((agent,i) => {
    if (!groups.has(agent.pos)) groups.set(agent.pos,[]);
    groups.get(agent.pos).push(`${id}:${i}`);
  });
  if (players) players.forEach((replay,id) => {
    const frames = replay.days[day - 1]?.frames ?? []; append(frames[Math.min(step,frames.length - 1)]?.agents ?? [],id);
  }); else append(fallback,player);
  for (const ids of groups.values()) ids.forEach((key,i) => {
    const angle = 2 * Math.PI * i / ids.length - Math.PI / 2, radius = ids.length > 1 ? 5.5 * size / 20 : 0;
    result.set(key,[Math.cos(angle) * radius,Math.sin(angle) * radius]);
  });
  return result;
}
function drawTerrainMark(ctx,terrain,cx,cy,k) {
  if (terrain === 2) {
    ctx.beginPath(); ctx.moveTo(cx - 6.5 * k,cy + 4.5 * k); ctx.lineTo(cx - 2 * k,cy - 4.5 * k); ctx.lineTo(cx + .5 * k,cy - .5 * k);
    ctx.lineTo(cx + 2.5 * k,cy - 3 * k); ctx.lineTo(cx + 6.5 * k,cy + 4.5 * k); ctx.closePath();
    ctx.fillStyle = 'rgba(236,240,255,0.15)'; ctx.fill();
  } else if (terrain === 3) {
    ctx.beginPath(); ctx.moveTo(cx - 6 * k,cy - 2 * k);
    ctx.quadraticCurveTo(cx - 3 * k,cy - 5 * k,cx,cy - 2 * k); ctx.quadraticCurveTo(cx + 3 * k,cy + k,cx + 6 * k,cy - 2 * k);
    ctx.moveTo(cx - 4.5 * k,cy + 3.5 * k); ctx.quadraticCurveTo(cx - 1.5 * k,cy + .5 * k,cx + 1.5 * k,cy + 3.5 * k);
    ctx.strokeStyle = 'rgba(120,180,255,0.4)'; ctx.lineWidth = 1.3 * k; ctx.stroke();
  }
}
function drawUdon(ctx,x,y,k) {
  ctx.beginPath();
  for (const row of [-2,1]) {
    ctx.moveTo(x - 4.2 * k,y + row * k);
    ctx.quadraticCurveTo(x - 2 * k,y + (row - 2.5) * k,x,y + row * k);
    ctx.quadraticCurveTo(x + 2 * k,y + (row + 2.5) * k,x + 4.2 * k,y + row * k);
  }
  ctx.strokeStyle = '#141a2e'; ctx.lineWidth = 1.3 * k; ctx.stroke();
}
function drawStocks(ctx,cx,y,k,left,total) {
  for (let j = 0; j < total; j++) {
    ctx.beginPath(); ctx.arc(cx + (j - (total - 1) / 2) * 4.6 * k,y,1.7 * k,0,Math.PI * 2);
    ctx.fillStyle = j < left ? '#ffd98a' : 'rgba(255,255,255,0.2)'; ctx.fill();
  }
}
function drawBrandLabel(ctx,cx,cy,k,brand) {
  // 左側の空きに置き、セル番号・うどん記号・在庫の点との重なりを避ける。
  ctx.beginPath(); roundedRect(ctx,cx - 16 * k,cy - 4.5 * k,10 * k,9 * k,2 * k);
  ctx.fillStyle = '#10162a'; ctx.fill();
  label(ctx,brand,cx - 11 * k,cy,6.5 * k,'#f8fafc',8 * k);
}

// 最終フレームには次の時間区間がないため数えない。移動中は判定上の出発セルに計上する。
const residenceCache = new WeakMap(), heatmapCache = new WeakMap();
export function residenceSteps(day,cellCount) {
  let cached = residenceCache.get(day);
  if (cached?.patrol.length === cellCount) return cached;
  cached = {patrol:Array(cellCount).fill(0),supply:Array(cellCount).fill(0)};
  for (let s = 0; s + 1 < day.frames.length; s++) {
    const frame = day.frames[s], duration = Math.max(0,(day.frames[s + 1].step ?? s + 1) - (frame.step ?? s));
    for (const agent of frame.agents) cached[agent.kind === 0 ? 'patrol' : 'supply'][agent.pos] += duration;
  }
  residenceCache.set(day,cached);
  return cached;
}
export function residenceHeatmap(day,cellCount,options) {
  const owner = options.matchReplay ?? options.heatmapDays ?? day;
  const allDays = options.heatmapScope === 'all', allTeams = options.heatmapTeams === 'all';
  const key = `${cellCount}:${allDays ? 'all' : day.day}:${allTeams ? 'all' : options.player ?? 0}`;
  let cache = heatmapCache.get(owner);
  if (!cache) {cache = new Map(); heatmapCache.set(owner,cache);}
  if (cache.has(key)) return cache.get(key);
  const players = options.matchReplay ? [options.matchReplay,...(options.matchReplay.opponents || [])] : [{days:options.heatmapDays ?? [day]}];
  const selected = allTeams ? players : [players[options.player ?? 0] ?? players[0]];
  const result = {patrol:Array(cellCount).fill(0),supply:Array(cellCount).fill(0),total:Array(cellCount).fill(0),maxPatrol:0,maxSupply:0,maxTotal:0};
  for (const player of selected) for (const d of player.days) {
    if (!allDays && d.day !== day.day) continue;
    const counts = residenceSteps(d,cellCount);
    for (let pos = 0; pos < cellCount; pos++) {result.patrol[pos] += counts.patrol[pos]; result.supply[pos] += counts.supply[pos];}
  }
  result.total = result.patrol.map((count,pos) => count + result.supply[pos]);
  result.maxPatrol = Math.max(...result.patrol); result.maxSupply = Math.max(...result.supply); result.maxTotal = Math.max(...result.total);
  cache.set(key,result);
  return result;
}
function heatmapColor(count,max,kind) {
  if (!count) return '#f1f5f9';
  const from = kind === 0 ? [239,246,255] : [255,247,237], to = kind === 0 ? [37,99,235] : [234,88,12];
  const ratio = Math.log1p(count) / Math.log1p(Math.max(1,max));
  return `rgb(${from.map((v,i) => Math.round(v + (to[i] - v) * ratio)).join(',')})`;
}
function drawHeatmapCell(ctx,cx,cy,cell,pos,mode,heatmap) {
  const key = mode === 'both' ? 'total' : mode;
  const max = mode === 'both' ? heatmap.maxTotal : mode === 'patrol' ? heatmap.maxPatrol : heatmap.maxSupply;
  hexPath(ctx,cx,cy,cell,20); ctx.fillStyle = heatmapColor(heatmap[key][pos],max,mode === 'supply' ? 1 : 0); ctx.fill();
}

// 日ごとの実際の移動を保存し、停止中の重複位置を省く。同じ日の再生中は経路を変えない。
const dayRoutes = new WeakMap();
// 参照ページと同じく、チーム色の色相を車両数に応じて最大140度の範囲に広げる。
function routeColor(color,id,count) {
  const rgb = parseInt(color.slice(1),16), r = (rgb >> 16 & 255) / 255, g = (rgb >> 8 & 255) / 255, b = (rgb & 255) / 255;
  const max = Math.max(r,g,b), min = Math.min(r,g,b), light = (max + min) / 2, delta = max - min;
  let hue = 0, saturation = 0;
  if (delta !== 0) {
    saturation = delta / (1 - Math.abs(2 * light - 1));
    hue = (max === r ? (g - b) / delta % 6 : max === g ? (b - r) / delta + 2 : (r - g) / delta + 4) * 60;
    if (hue < 0) hue += 360;
  }
  const span = Math.min(46 * (count - 1),140), shift = count > 1 ? span * id / (count - 1) - span / 2 : 0;
  return `hsl(${((hue + shift + 360) % 360).toFixed(1)}, ${(saturation * 100).toFixed(0)}%, ${(light * 100).toFixed(0)}%)`;
}
export function drawDayRoutes(ctx,center,size,day,agentColor) {
  if (!day?.frames?.length) return;
  let routes = dayRoutes.get(day);
  if (!routes) {
    routes = day.frames[0].agents.map((agent,id) => {
      const positions = [];
      for (const frame of day.frames) {
        const pos = frame.agents[id].pos;
        if (positions.at(-1) !== pos) positions.push(pos);
      }
      return {agent,id,positions};
    });
    dayRoutes.set(day,routes);
  }
  const k = size / 20;
  ctx.save(); ctx.lineJoin = ctx.lineCap = 'round';
  ctx.lineWidth = 1.4 * k; ctx.setLineDash([3 * k,4 * k]); ctx.globalAlpha = .65;
  for (const {agent,id,positions} of routes) {
    if (positions.length < 2) continue;
    ctx.beginPath();
    positions.forEach((pos,i) => {const [x,y] = center(pos); if (i === 0) ctx.moveTo(x,y); else ctx.lineTo(x,y);});
    ctx.strokeStyle = routeColor(agentColor(agent),id,routes.length); ctx.stroke();
  }
  ctx.restore();
}

// 予約が始まった時点を日ごとに記録し、複数ステップの移動も一定速度で補間する。
const dayMotions = new WeakMap();
export function agentPosition(center,day,step,id,progress = 0) {
  const frame = day.frames[step], agent = frame.agents[id];
  let motions = dayMotions.get(day);
  if (!motions) {
    motions = [];
    day.frames.forEach((f,s) => {
      motions.push(f.agents.map((a,i) => {
        const pending = f.pending?.[i], previous = motions[s - 1]?.[i];
        if (pending?.destination == null) return null;
        if (previous?.from === a.pos && previous.to === pending.destination && previous.until === pending.until) return previous;
        return {from:a.pos,to:pending.destination,start:f.step ?? s,until:pending.until};
      }));
    });
    dayMotions.set(day,motions);
  }
  const motion = motions[step][id];
  const from = center(agent.pos), to = center(motion?.to ?? day.frames[step + 1]?.agents[id]?.pos ?? agent.pos);
  const ratio = motion && motion.until > motion.start
    ? Math.max(0,Math.min(1,((frame.step ?? step) + progress - motion.start) / (motion.until - motion.start))) : progress;
  return [from[0] + (to[0] - from[0]) * ratio,from[1] + (to[1] - from[1]) * ratio];
}
// view: {frame, day, s, progress, roadStatus}。options: 表示設定、agentColor(agent)、routeOverlay / overlay(ctx,center,size)。
export function renderBoard(canvas,problem,{frame,day,s = 0,progress = 0,roadStatus},options) {
  const ctx = canvas.getContext('2d'), L = layout(problem,canvas.width - 8,canvas.height - 8), u = L.s, k = u / 20, cell = 19.5 * k;
  const center = p => L.center(Math.floor(p / L.W),p % L.W);
  const heatmap = ['both','patrol','supply'].includes(options.heatmapMode) ? residenceHeatmap(day,L.H * L.W,options) : null;
  // 塗りつぶしを描画後の1pxだけ重ね、アンチエイリアスによる暗い継ぎ目もなくす。
  const heatmapCell = u + 1 / (options.camera?.zoom ?? 1);
  ctx.fillStyle = '#0d1020'; ctx.fillRect(0,0,canvas.width,canvas.height); ctx.save();
  if (options.camera) {
    const {zoom,x,y} = options.camera;
    ctx.translate(canvas.width / 2 + x,canvas.height / 2 + y); ctx.scale(zoom,zoom); ctx.translate(-canvas.width / 2,-canvas.height / 2);
  }
  ctx.translate(4,4);
  ctx.lineJoin = 'miter'; ctx.lineCap = 'round';
  for (let r = 0; r < L.H; r++) for (let c = 0; c < L.W; c++) {
    const terrain = problem.map.cells[r][c], [cx,cy] = L.center(r,c);
    if (heatmap && terrain !== 3) {drawHeatmapCell(ctx,cx,cy,heatmapCell,r * L.W + c,options.heatmapMode,heatmap); continue;}
    hexPath(ctx,cx,cy,heatmap ? heatmapCell : cell,heatmap ? 20 : 19.5); ctx.fillStyle = heatmap ? '#cbd5e1' : terrain === 1 ? ROAD_COLOR[0] : TERRAIN_COLOR[terrain]; ctx.fill();
    if (!heatmap) {ctx.strokeStyle = RIM; ctx.lineWidth = 1.1 * k; ctx.stroke();}
    drawTerrainMark(ctx,terrain,cx,cy,k);
    const status = roadStatus.get(r * L.W + c) ?? 0;
    if (!heatmap && terrain === 1 && status > 0) {
      const width = 2.4 * k;
      // 線の中心を同心の六角形上に置き、マイター結合を含めた外端をセル内に収める。
      // 隣のセルの塗りつぶし・枠による上書きを防ぎ、描画順によらず六辺の太さを保つ。
      hexPath(ctx,cx,cy,cell - width / Math.sqrt(3),19.5);
      ctx.strokeStyle = ROAD_COLOR[status]; ctx.lineWidth = width; ctx.stroke();
    }
  }
  if (options.showIds && !heatmap) for (let pos = 0; pos < L.H * L.W; pos++) {const [cx,cy] = center(pos); label(ctx,pos,cx,cy - u * .66,Math.max(8,u * .28),'#cbd5e1',u * 1.3);}
  if (!heatmap && options.selectedCell != null) {
    const [x,y] = center(options.selectedCell); hexPath(ctx,x,y,cell,19.5); ctx.strokeStyle = SELECT; ctx.lineWidth = 1.6 * k; ctx.stroke();
  }
  if (heatmap) {ctx.restore(); return;}
  // 全車両の当日全体の経路を参照ページと同じ点線で表示し、店舗・車の下に重ねる。
  if (options.showRoute) {
    options.routeOverlay?.(ctx,center,u);
    drawDayRoutes(ctx,center,u,day,options.agentColor);
  }
  // 店舗はセル中央。参照ページと同じうどん記号と4.6間隔の在庫の点を描く。
  const acquired = new Set(frame.events.filter(e => e.kind === 'collect').map(e => e.spot));
  problem.spots.forEach((spot,i) => {
    const [cx,cy] = center(spot.pos), left = Math.min(frame.stocks[i],spot.stocks);
    ctx.globalAlpha = left > 0 ? 1 : .4;
    hexPath(ctx,cx,cy,7.8 * k,7.8); ctx.fillStyle = brandColor(spot.brand); ctx.fill();
    ctx.strokeStyle = 'rgba(7,11,22,0.85)'; ctx.lineWidth = 1.4 * k; ctx.stroke();
    ctx.globalAlpha = 1; drawUdon(ctx,cx,cy,k); drawStocks(ctx,cx,cy + 13.2 * k,k,left,spot.stocks);
    if (acquired.has(i)) circle(ctx,cx,cy,9 * k,ACQUIRED,2 * k);
  });
  // 同一セルの車は半径5.5の円周に配置。店舗位置による追加のずらしは行わない。
  const refilled = new Set(frame.events.filter(e => e.kind === 'refill').map(e => e.agent));
  const offsets = boardAgentOffsets(options,day.day,s,u,frame.agents), nextOffsets = boardAgentOffsets(options,day.day,s + 1,u,day.frames[s + 1]?.agents ?? frame.agents);
  frame.agents.forEach((a,i) => {
    const [cx,cy] = agentPosition(center,day,s,i,progress), rad = 6.3 * k;
    const key = `${options.player ?? 0}:${i}`, [ox,oy] = offsets.get(key), [nx,ny] = nextOffsets.get(key);
    const x = cx + ox + (nx - ox) * progress, y = cy + oy + (ny - oy) * progress;
    if (options.selectedAgent === i) circle(ctx,x,y,rad + 3.2 * k,SELECT,2.2 * k);
    drawVehicle(ctx,x,y,u,a,options.agentColor(a),i,problem.fuelLimits);
    if (refilled.has(i)) circle(ctx,x,y,7 * k,REFUEL,1.6 * k);
  });
  options.overlay?.(ctx,center,u);
  // 系列色は10色で循環するため番号も併記する。車両の上に描き、停車中も識別できる。
  problem.spots.forEach(spot => {const [cx,cy] = center(spot.pos); drawBrandLabel(ctx,cx,cy,k,spot.brand);});
  ctx.restore();
}
