// Server-only match UI, adapted from tester/game/web/match-ui.js for 1〜16 teams.
// Teams are peers; the board, history lifecycle, navigation and GIF codec are shared.
import {drawVehicle,boardAgentOffsets,drawDayRoutes,agentPosition} from './board.js';
export const PLAYER_COLORS = ['#2563eb','#b45309','#15803d','#9333ea','#e11d48','#0891b2','#be123c','#4d7c0f',
  '#7c3aed','#b91c1c','#0f766e','#a16207','#6d28d9','#dc2626','#065f46','#c2410c'];
export const MAX_PLAYERS = PLAYER_COLORS.length;
export const RANK_SCOPES = [['','全体'],['first','1回戦'],['semifinal','準決勝'],['final','決勝']];
export const playerColor = id => PLAYER_COLORS[id % MAX_PLAYERS];
const validRank = (rank,count) => Number.isInteger(rank) && rank >= 1 && rank <= count;
const compareScores = (a,b) => a.uniqueBrands-b.uniqueBrands || a.dailyBrands-b.dailyBrands || a.udon-b.udon;
export const averageRankText = rank => rank == null ? '—' : `${rank.toFixed(2)}位`;
export function playerReplays(replay) { return replay ? [replay,...(replay.opponents || [])] : []; }
export function teamCount(cases) { return cases.reduce((n,c) => Math.max(n,c.players?.length || 0),0); }
// Ranks use all-AC matches only, as in the game tester. versus[i-1] is team 0 against team i.
export function matchStats(cases, count = teamCount(cases)) {
  const ranks = Array.from({length:count},() => Array(count).fill(0)), sums = Array(count).fill(0);
  const versus = Array.from({length:Math.max(0,count-1)},() => [0,0,0]); let matches = 0;
  for (const c of cases) {
    if (c.status !== 'AC' || c.players?.length !== count || !c.players.every(p => p.status === 'AC' && validRank(p.rank,count))) continue;
    matches++;
    c.players.forEach((p,i) => {sums[i] += p.rank; ranks[i][p.rank-1]++;});
    for (let i = 1; i < count; i++) {
      const d = compareScores(c.players[0].score,c.players[i].score);
      versus[i-1][d > 0 ? 0 : d < 0 ? 2 : 1]++;
    }
  }
  return {count:matches,ranks,versus,averageRanks:sums.map(sum => matches ? sum/matches : null)};
}
export function matchScopeStats(cases, count = teamCount(cases)) {
  return RANK_SCOPES.map(([format,label]) => ({format,label,...matchStats(cases.filter(c => !format || c.format === format),count)}));
}
export function rankBadge(element, rank, tied = false, count = MAX_PLAYERS) {
  const valid = validRank(rank,count);
  return element('span',valid ? `${tied ? '同率' : ''}${rank}位` : '—',`rank-badge rank-${valid && rank <= 5 ? rank : 'none'}`);
}
export function caseRankBadge(element, c, player = 0) {
  const rank = c.players?.[player]?.rank, count = c.players?.length || 0;
  const badge = rankBadge(element,rank,c.players?.filter(p => p.rank === rank).length > 1,count);
  badge.title = c.status === 'AC' ? `最終順位 / ${count}チーム` : '代替動作を含む参考順位。平均順位の集計対象外です。';
  if (c.status !== 'AC' && rank != null) badge.append(element('small','参考'));
  return badge;
}
// One results-table cell: rank, the three score values and the team's own verdict.
export function teamCell(element, c, player) {
  const cell = element('td',undefined,'team-cell'), p = c.players?.[player];
  if (!p) {cell.append(element('span','—','note')); return cell;}
  cell.append(caseRankBadge(element,c,player),element('span',`${p.score.uniqueBrands} · ${p.score.dailyBrands} · ${p.score.udon}`,'team-score'));
  if (p.status !== 'AC') cell.append(element('small',p.status,'warn'));
  cell.title = `種類数 ${p.score.uniqueBrands} · 日別種類 ${p.score.dailyBrands} · 玉数 ${p.score.udon} · 回答 ${p.answerTimeMs} ms · ${p.status}`;
  return cell;
}
export function rankSummary(element, stats, label, player = 0) {
  const card = element('section',undefined,'rank-summary');
  card.append(element('h3',label),element('span','チーム0の平均順位','note'),element('strong',averageRankText(stats.averageRanks[player] ?? null),'rank-average'),element('span',`全員AC ${stats.count}試合`,'note'));
  const distribution = element('div',undefined,'rank-distribution');
  (stats.ranks[player] || []).forEach((count,i) => {
    const item = element('div',undefined,`rank-count rank-${i < 5 ? i+1 : 'none'}`);
    item.append(element('span',`${i+1}位`),element('b',`${count}`),element('small','回'));
    distribution.append(item);
  });
  card.append(distribution);
  return card;
}
export function opponentBreakdown(element, stats, names) {
  const details = element('details',undefined,'rank-opponents'); details.append(element('summary','チーム0から見た他チームとの勝・同点・敗'));
  stats.versus.forEach((v,i) => details.append(element('p',`${i+1}: ${names[i+1] ?? `チーム${i+1}`}: ${v[0]}勝 ${v[1]}同点 ${v[2]}敗`,'note')));
  if (!stats.versus.length) details.append(element('p','他のチームはありません。','note'));
  return details;
}
function teamHeading(element, name, id) {
  const th = element('th',`${id}: ${name}`); th.style.color = playerColor(id); return th;
}
export function renderMatchSummary(container, cases, names, element) {
  const scopes = matchScopeStats(cases,names.length), grid = element('div',undefined,'rank-tables');
  const scroll = element('div',undefined,'table-scroll'), table = element('table',undefined,'data-table totals-table rank-averages');
  const head = element('thead'), row = element('tr'), body = element('tbody');
  table.append(element('caption','チーム別の平均順位'));
  row.append(element('th','形式'),element('th','AC試合'),...names.map((name,id) => teamHeading(element,name,id))); head.append(row);
  for (const s of scopes) {
    const r = element('tr',undefined,s.format ? '' : 'total-row');
    r.append(element('td',s.label),element('td',s.count),...s.averageRanks.map(rank => element('td',averageRankText(rank))));
    body.append(r);
  }
  table.append(head,body); scroll.append(table);
  const distributionScroll = element('div',undefined,'table-scroll'), distribution = element('table',undefined,'data-table rank-by-format');
  distribution.setAttribute('aria-label','チーム別の順位分布');
  const distributionHead = element('thead'), headings = element('tr'), distributionBody = element('tbody');
  for (const label of ['チーム','平均順位',...names.map((_,i) => `${i+1}位`)]) headings.append(element('th',label));
  distributionHead.append(headings);
  names.forEach((name,id) => {
    const r = element('tr'), label = element('td',`${id}: ${name}`); label.style.color = playerColor(id);
    r.append(label,element('td',averageRankText(scopes[0].averageRanks[id]),'mean-rank'));
    for (const count of scopes[0].ranks[id] || []) {const cell = element('td',count,'rank-frequency'); cell.title = `${count}回`; r.append(cell);}
    distributionBody.append(r);
  });
  distribution.append(distributionHead,distributionBody); distributionScroll.append(distribution);
  const details = element('details',undefined,'rank-format-details');
  details.open = container.querySelector('.rank-format-details')?.open ?? false;
  details.append(element('summary',`チーム別の順位分布（全体 · 全員AC ${scopes[0].count}試合）`),distributionScroll);
  grid.append(scroll,details);
  container.replaceChildren(grid);
}
// Score totals of every team over all completed matches, including non-AC ones.
export function teamTotalsTable(element, cases, names, compact = false) {
  const table = element('table',undefined,`data-table totals-table${compact ? ' compact-totals' : ''}`), head = element('thead'), body = element('tbody'), heading = element('tr');
  for (const text of ['チーム',...(compact ? [] : ['AC']),'種類数','日別種類','玉数']) heading.append(element('th',text));
  head.append(heading);
  names.forEach((name,id) => {
    const rows = cases.map(c => c.players?.[id]).filter(Boolean), row = element('tr');
    const sum = key => rows.reduce((total,p) => total + p.score[key],0).toLocaleString('ja-JP');
    const label = element('td',`${id}: ${name}`); label.style.color = playerColor(id);
    row.append(label,...(compact ? [] : [element('td',`${rows.filter(p => p.status === 'AC').length} / ${rows.length}`)]),element('td',sum('uniqueBrands')),element('td',sum('dailyBrands')),element('td',sum('udon')));
    body.append(row);
  });
  table.append(head,body); return table;
}
export function createTeamFields(container, count, names, element) {
  const previous = readTeamNames(container);
  container.replaceChildren(...Array.from({length:count},(_,id) => {
    const label = element('label',`チーム${id}（token-p${id}）`), input = element('input');
    label.style.color = playerColor(id);
    input.type = 'text'; input.maxLength = 80; input.dataset.team = id; input.placeholder = `チーム${id}`;
    input.value = previous[id] ?? names[id] ?? '';
    label.append(input); return label;
  }));
}
export function readTeamNames(container) {
  return [...container.querySelectorAll('input[data-team]')].map(input => input.value.trim());
}
export function drawOtherPlayerRoutes(ctx, center, size, settings, day) {
  if (!settings.showRoute || !settings.showOthers || !settings.matchReplay) return;
  playerReplays(settings.matchReplay).forEach((r,id) => {
    if (id === settings.player) return;
    drawDayRoutes(ctx,center,size,r.days.find(d => d.day === day),() => playerColor(id));
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
      drawVehicle(ctx,cx + ox + (nx - ox) * fraction,cy + oy + (ny - oy) * fraction,size,agent,playerColor(id),i,settings.fuelLimits);
    });
  });
}

export function renderPlayerCards({container,replay,names,player,day,step,element,onSelect}) {
  const players = playerReplays(replay), scores = players.map(p => p.score);
  container.replaceChildren(...players.map((r,id) => {
    const frames = r.days[day]?.frames || [], score = frames[Math.min(step,frames.length-1)]?.score || r.score;
    const rank = 1+scores.filter(b => compareScores(b,r.score) > 0).length;
    const button = element('button',undefined,'match-player');
    button.style.setProperty('--player-color',playerColor(id));
    button.setAttribute('aria-pressed',String(id === player));
    const failures = [r.selectionFailure,...r.days.map(d => d.failure)].filter(Boolean);
    const verdict = failures.some(f => f.kind === 'timeout') ? 'TLE' : failures.some(f => f.kind === 'process') ? 'RE' : failures.length ? 'WA' : 'AC';
    const tied = scores.filter(s => compareScores(s,r.score) === 0).length > 1;
    const heading = element('div',undefined,'player-rank-heading');
    heading.append(rankBadge(element,rank,tied,players.length),element('small','最終順位'));
    button.append(heading,element('strong',`${id}: ${names[id] ?? `チーム${id}`}`),element('span',`種類 ${score.uniqueBrands} · 日別 ${score.dailyBrands} · 玉 ${score.udon}`),element('small',verdict));
    button.onclick = () => onSelect(id);
    return button;
  }));
}
