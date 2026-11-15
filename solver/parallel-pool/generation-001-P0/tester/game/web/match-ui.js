// Match-only UI behavior. The board, history lifecycle, navigation and GIF codec are shared.
import {drawVehicle,boardAgentOffsets,drawDayRoutes,agentPosition} from './board.js';
export const PLAYER_COLORS = ['#2563eb','#b45309','#15803d','#9333ea','#e11d48'];
export const RANK_SCOPES = [['','全体'],['first','1回戦'],['semifinal','準決勝'],['final','決勝']];
const validRank = rank => Number.isInteger(rank) && rank >= 1 && rank <= 5;
export const averageRankText = rank => rank == null ? '—' : `${rank.toFixed(2)}位`;
export function playerReplays(replay) { return replay?.opponents?.length === 4 ? [replay,...replay.opponents] : []; }
export function matchStats(cases) {
  const ranks = [0,0,0,0,0], sums = [0,0,0,0,0], opponents = Array.from({length:4},() => [0,0,0]); let count = 0;
  for (const c of cases) {
    if (c.status !== 'AC' || c.players?.length !== 5 || !c.players.every(p=>p.status === 'AC' && validRank(p.rank))) continue;
    count++; ranks[c.players[0].rank - 1]++;
    c.players.forEach((p,i)=>{sums[i]+=p.rank;});
    for (let i = 1; i < 5; i++) {
      const a = c.players[0].score,b = c.players[i].score;
      const d = a.uniqueBrands-b.uniqueBrands || a.dailyBrands-b.dailyBrands || a.udon-b.udon;
      opponents[i-1][d>0 ? 0 : d<0 ? 2 : 1]++;
    }
  }
  const playerAverageRanks = sums.map(sum=>count ? sum/count : null);
  return {count,ranks,opponents,averageRank:playerAverageRanks[0],playerAverageRanks};
}
export function matchScopeStats(cases) {
  return RANK_SCOPES.map(([format,label])=>({format,label,...matchStats(cases.filter(c=>!format || c.format===format))}));
}
export function rankBadge(element, rank, tied = false) {
  return element('span',validRank(rank) ? `${tied ? '同率' : ''}${rank}位` : '—',`rank-badge rank-${validRank(rank) ? rank : 'none'}`);
}
export function caseRankBadge(element, c) {
  const rank=c.players?.[0]?.rank;
  const badge=rankBadge(element,rank,c.players?.filter(p=>p.rank===rank).length>1);
  badge.title=c.status === 'AC' ? '対象ソルバの最終順位 / 5人' : '代替動作を含む参考順位。平均順位の集計対象外です。';
  if(c.status !== 'AC') badge.append(element('small','参考'));
  return badge;
}
export function rankSummary(element, stats, label) {
  const card=element('section',undefined,'rank-summary');
  card.append(element('h3',label),element('span','対象の平均順位','note'),element('strong',averageRankText(stats.averageRank),'rank-average'),element('span',`全員AC ${stats.count}試合`,'note'));
  const distribution=element('div',undefined,'rank-distribution');
  stats.ranks.forEach((count,i)=>{
    const item=element('div',undefined,`rank-count rank-${i+1}`);
    item.append(element('span',`${i+1}位`),element('b',`${count}`),element('small','回'));
    distribution.append(item);
  });
  card.append(distribution);
  return card;
}
export function opponentBreakdown(element, stats, names) {
  const details=element('details',undefined,'rank-opponents');details.append(element('summary','対戦相手別の勝・同点・敗'));
  stats.opponents.forEach((v,i)=>details.append(element('p',`${names[i+1]}: ${v[0]}勝 ${v[1]}同点 ${v[2]}敗`,'note')));
  return details;
}
export function renderMatchSummary(container, cases, names, element) {
  const scopes=matchScopeStats(cases), grid=element('div',undefined,'rank-tables');
  const targetScroll=element('div',undefined,'table-scroll'), target=element('table',undefined,'data-table rank-by-format');
  target.setAttribute('aria-label','対象ソルバ · 形式別の順位');
  const targetHead=element('thead'), headings=element('tr'), targetBody=element('tbody');
  for(const label of ['形式','AC試合','平均順位','1位','2位','3位','4位','5位']) headings.append(element('th',label));
  targetHead.append(headings);
  for(const s of scopes) {
    const r=element('tr',undefined,s.format ? '' : 'total-row');
    r.append(element('td',s.label),element('td',s.count),element('td',averageRankText(s.averageRank),'mean-rank'));
    for(const count of s.ranks) {const cell=element('td',count,'rank-frequency');cell.title=`${count}回`;r.append(cell);}
    targetBody.append(r);
  }
  target.append(targetHead,targetBody);targetScroll.append(target);
  const scroll=element('div',undefined,'table-scroll'), table=element('table',undefined,'data-table totals-table rank-averages');
  const head=element('thead'), row=element('tr'), body=element('tbody');
  table.append(element('caption','プレイヤー別の平均順位'));
  row.append(element('th','形式'),element('th','AC試合'),...names.map((name,id)=>element('th',`${id}: ${name}`)));head.append(row);
  for(const s of scopes) {
    const r=element('tr',undefined,s.format ? '' : 'total-row');
    r.append(element('td',s.label),element('td',s.count),...s.playerAverageRanks.map(rank=>element('td',averageRankText(rank))));body.append(r);
  }
  table.append(head,body);scroll.append(table);
  const details=element('details',undefined,'rank-format-details');
  details.open=container.querySelector('.rank-format-details')?.open ?? false;
  details.append(element('summary','対象ソルバ · 形式別の順位'),targetScroll);
  grid.append(scroll,details);
  container.replaceChildren(grid,opponentBreakdown(element,scopes[0],names));
}
export function createOpponentFields(container, element) {
  for (let id=1;id<=4;id++) {
    const box=element('fieldset'),legend=element('legend',`プレイヤー${id} · 対戦相手`);
    legend.style.color=PLAYER_COLORS[id];box.append(legend);
    for (const [key,label] of [['name','表示名'],['version','実装バージョン'],['solver','実行コマンド（JSON）'],['build','ビルドコマンド（JSON / nullで省略）']]) {
      const l=element('label',label),input=element(['solver','build'].includes(key)?'textarea':'input');input.id=`opponent-${id}-${key}`;if(input.tagName==='TEXTAREA')input.rows=5;
      l.append(input);box.append(l);
    }
    const l=element('label','内部並列'),select=element('select');select.id=`opponent-${id}-parallel`;
    for(const [value,label] of [['false','なし'],['true','あり']]) {const o=element('option',label);o.value=value;select.append(o);}
    l.append(select);box.append(l);container.append(box);
  }
}
export function fillOpponents($, config) {
  config.opponents?.forEach((p,i)=>{
    for(const k of ['name','version']) $(`opponent-${i+1}-${k}`).value=p[k]||'';
    for(const k of ['solver','build']) $(`opponent-${i+1}-${k}`).value=JSON.stringify(p[k]??null,null,2);
    $(`opponent-${i+1}-parallel`).value=String(p.solverParallel??false);
  });
}
export function readOpponents($) {
  return Array.from({length:4},(_,i)=>({name:$(`opponent-${i+1}-name`).value.trim(),version:$(`opponent-${i+1}-version`).value.trim(),solver:JSON.parse($(`opponent-${i+1}-solver`).value),build:JSON.parse($(`opponent-${i+1}-build`).value),solverParallel:$(`opponent-${i+1}-parallel`).value==='true'}));
}
export function drawOtherPlayerRoutes(ctx, center, size, settings, day) {
  if (!settings.showRoute || !settings.showOthers || !settings.matchReplay) return;
  playerReplays(settings.matchReplay).forEach((r,id) => {
    if (id === settings.player) return;
    drawDayRoutes(ctx,center,size,r.days.find(d => d.day === day),() => PLAYER_COLORS[id]);
  });
}
export function drawOtherPlayers(ctx,center,size,settings,day,step,progress = 0) {
  if (!settings.showOthers || !settings.matchReplay) return;
  const offsets = boardAgentOffsets(settings,day,step,size), nextOffsets = boardAgentOffsets(settings,day,step + 1,size);
  playerReplays(settings.matchReplay).forEach((replay,id) => {
    if (id === settings.player) return;
    const replayDay = replay.days[day - 1], frames = replayDay?.frames ?? [], s = Math.min(step,frames.length - 1);
    (frames[s]?.agents ?? []).forEach((agent,i) => {
      const key = `${id}:${i}`, [ox,oy] = offsets.get(key), [nx,ny] = nextOffsets.get(key), fraction = s < frames.length - 1 ? progress : 0;
      const [cx,cy] = agentPosition(center,replayDay,s,i,fraction);
      drawVehicle(ctx,cx + ox + (nx - ox) * fraction,cy + oy + (ny - oy) * fraction,size,agent,PLAYER_COLORS[id],i,settings.fuelLimits);
    });
  });
}

export function renderPlayerCards({container,replay,names,player,day,step,element,onSelect}) {
  const players=playerReplays(replay), scores=players.map(p=>p.score);
  container.replaceChildren(...players.map((r,id)=>{
    const frame=r.days[day]?.frames[step], score=frame?.score || r.score;
    const rank=1+scores.filter(b=>(b.uniqueBrands-r.score.uniqueBrands || b.dailyBrands-r.score.dailyBrands || b.udon-r.score.udon)>0).length;
    const button=element('button',undefined,'match-player');
    button.style.setProperty('--player-color',PLAYER_COLORS[id]);
    button.setAttribute('aria-pressed',String(id===player));
    const failures=[r.selectionFailure,...r.days.map(d=>d.failure)].filter(Boolean);
    const verdict=failures.some(f=>f.kind==='timeout')?'TLE':failures.some(f=>f.kind==='process')?'RE':failures.length?'WA':'AC';
    const tied=scores.filter(s=>s.uniqueBrands===r.score.uniqueBrands && s.dailyBrands===r.score.dailyBrands && s.udon===r.score.udon).length>1;
    const heading=element('div',undefined,'player-rank-heading');
    heading.append(rankBadge(element,rank,tied),element('small','最終順位'));
    button.append(heading,element('strong',`${id}: ${names[id]}`),element('span',`種類 ${score.uniqueBrands} · 日別 ${score.dailyBrands} · 玉 ${score.udon}`),element('small',verdict));
    button.onclick=()=>onSelect(id);
    return button;
  }));
}
