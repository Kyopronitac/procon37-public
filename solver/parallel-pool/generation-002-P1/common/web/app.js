'use strict';
import {Navigation,parseRoute,withoutDeletedRuns} from './navigation.js';
import {SCORE_KEYS,scoreTotals,scoreOrder,allAccepted,winCounts} from './record-stats.js';
import {playbackChapters,chapterProgress,advancePlayback} from './playback.js';
import {planGifFrames,exportGif} from './gif-export.js';
import {rerunConfig} from './run-config.js';
import {AGENT_COLOR,BOARD_LEGEND,boardPlayerColor,brandColor,layout,renderBoard,residenceHeatmap} from './board.js';
import {PLAYER_COLORS,playerReplays,matchStats,matchScopeStats,averageRankText,rankSummary,caseRankBadge,opponentBreakdown,renderMatchSummary,createOpponentFields,fillOpponents,readOpponents,drawOtherPlayerRoutes,drawOtherPlayers,renderPlayerCards} from './match-ui.js';
const isGame = document.body.dataset.game === 'true';
const storagePrefix = isGame ? 'hexaudon.game' : 'hexaudon';

// 盤面の描画は board.js が担当する。
// 生成・判定は Rust の API が担当し、このファイルは保存状態を表示する。
import {bindBoardControls,boardPoint} from './viewer.js';
const $ = id => document.getElementById(id);
const camera = {zoom:1,x:0,y:0};
const FORMAT = {first:'1回戦', semifinal:'準決勝', final:'決勝'};
const OUTCOMES = [['wins','勝','win'],['draws','同点','draw'],['losses','敗','loss']];
const STATUS = {building:'ビルド中', running:'実行中', completed:'完了', failed:'失敗', cancelled:'中断'};
const state = {case:null, replay:null, days:[], timeline:[], chapters:[], t:0, progress:0, playing:false, heatmapShown:false, deleteTarget:null, deletedIds:new Set(),
  matchReplay:null, player:0, selectedAgent:null, selectedCell:null, activeId:null, selectedRunId:null, pollBusy:false,
  inspectorTab:'score',inspectorOpen:window.innerWidth >= 1000,gifAbort:null,gifUrl:null,
  exporting:new Set(),importing:false,localConfig:null,
  runs:[], runView:null, openedSeed:null, view:'results', runRequest:0, replayRequest:0, submitting:false, compareIds:[], comparison:null, availableCpus:1, compareMode:false, navigation:null, imports:new Map()};
function element(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}
function error(message) {$('error').textContent = message || ''; $('error').hidden = !message;}
function guard(fn) {return async (...args) => {try {error(''); await fn(...args);} catch (e) {error(e.message || String(e));}};}
async function api(path, body, method = body === undefined ? 'GET' : 'POST') {
  const options = body === undefined ? {method} : {method, headers:{'Content-Type':'application/json'}, body:JSON.stringify(body)};
  const response = await fetch(path, options);
  const raw = await response.text();
  let result;
  try {result = JSON.parse(raw);} catch {throw new Error(raw || `HTTP ${response.status}`);}
  if (!response.ok) throw Object.assign(new Error(result.error || `HTTP ${response.status}`),{status:response.status,path});
  return result;
}
function seedInput(id = 'seed') {
  const seed = $(id).value.trim();
  if (!/^\d+$/.test(seed) || BigInt(seed) > 18446744073709551615n) throw new Error('seedは0〜18446744073709551615の整数を指定してください。');
  return BigInt(seed).toString();
}
function integer(id, min, max) {
  const value = Number($(id).value);
  if (!Number.isInteger(value) || value < min || value > max) throw new Error(`${$(id).labels?.[0]?.textContent || id}は${min}〜${max}の整数を指定してください。`);
  return value;
}
function view(name) {
  if ((name === 'map' && !state.case) || (name !== 'map' && !state.runView)) return;
  if (name !== 'map') setPlaying(false);
  state.view = name;
  document.body.classList.toggle('replay-mode',name === 'map');
  for (const tab of document.querySelectorAll('[data-view]')) {
    tab.classList.toggle('active', tab.dataset.view === name);
    tab.setAttribute('aria-current', tab.dataset.view === name ? 'page' : 'false');
  }
  for (const key of ['map','results','logs','compare']) $(`${key}View`).hidden = key !== name;
}
function download(name, data) {
  const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], {type:'application/json'}));
  const link = element('a'); link.href = url; link.download = name; document.body.append(link); link.click(); link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function validateProblem(problem) {
  const map = problem?.map;
  if (!map || !Number.isInteger(map.width) || !Number.isInteger(map.height) || map.width < 1 || map.width > 32 || map.height < 1 || map.height > 32 ||
      !Array.isArray(map.cells) || map.cells.length !== map.height || map.cells.some(row => !Array.isArray(row) || row.length !== map.width || row.some(t => !Number.isInteger(t) || t < 0 || t > 3))) throw new Error('問題JSONの地形配列が不正です。');
  const pos = p => Number.isInteger(p) && p >= 0 && p < map.width * map.height;
  if (!Array.isArray(problem.agents) || !problem.agents.length || problem.agents.length > 8 || !problem.agents.every(pos) ||
      !Array.isArray(problem.spots) || problem.spots.length > 32 || problem.spots.some(s => !pos(s.pos) || !Number.isInteger(s.brand) || s.brand < 0 || !Number.isInteger(s.stocks) || s.stocks < 1) ||
      !Number.isFinite(problem.fuelLimits) || problem.fuelLimits < 1 || !Array.isArray(problem.daySteps) || !problem.daySteps.length || problem.daySteps.length > 10) throw new Error('問題JSONのエージェント・スポット・日数が不正です。');
}
function loadCase(testCase, replay = null, player = 0) {
  state.matchReplay = replay?.opponents?.length === 4 ? replay : null;
  state.player = state.matchReplay ? player : 0;
  if (isGame && replay && !state.matchReplay) throw new Error('5人対戦のリプレイを指定してください。');
  if (!isGame && state.matchReplay) throw new Error('対戦リプレイはgameの画面で開いてください。');
  if (state.matchReplay) replay = playerReplays(state.matchReplay)[state.player];
  if (isGame) {
    const names = playerNames();
    $('playerSelect').replaceChildren(...names.map((name,id) => {const o=element('option',`${id}: ${name}`);o.value=id;return o;}));
    $('playerSelect').value=state.player;
    $('playerSelect').disabled=!state.matchReplay;
  }
  validateProblem(testCase.problem);
  const problem = testCase.problem;
  const days = replay ? replay.days : [{day:1, steps:0, traffics:[], elapsedMs:0, frames:[{
    step:0, agents:problem.agents.map(pos => ({kind:0,pos,fuel:problem.fuelLimits})),
    stocks:problem.spots.map(s => s.stocks), pending:problem.agents.map(() => null),
    score:{uniqueBrands:0,dailyBrands:0,udon:0}, events:[]
  }]}];
  if (!Array.isArray(days) || !days.length || days.length > 10) throw new Error('リプレイの日数が不正です。');
  const timeline = [], seen = new Set(); let answerMs = 0;
  for (let d = 0; d < days.length; d++) {
    const day = days[d], daily = new Set();
    if (!Array.isArray(day.frames) || !day.frames.length || day.frames.length > 257 || !Array.isArray(day.traffics)) throw new Error('リプレイのフレームが不正です。');
    for (let s = 0; s < day.frames.length; s++) {
      const frame = day.frames[s];
      if (!frame || !Array.isArray(frame.agents) || frame.agents.length !== problem.agents.length ||
          frame.agents.some(a => ![0,1].includes(a.kind) || !Number.isInteger(a.pos) || a.pos < 0 || a.pos >= problem.map.width * problem.map.height || !Number.isFinite(a.fuel)) ||
          !Array.isArray(frame.stocks) || frame.stocks.length !== problem.spots.length || !Array.isArray(frame.pending) || frame.pending.length !== frame.agents.length || !Array.isArray(frame.events) || !frame.score) throw new Error('リプレイの状態配列が不正です。');
      for (const e of frame.events) if (e.kind === 'collect') {seen.add(e.brand); daily.add(e.brand);}
      if (s === day.frames.length - 1 && !day.failure) answerMs += day.elapsedMs;
      timeline.push({d,s,brands:new Set(seen),daily:new Set(daily),answerMs});
    }
  }
  setPlaying(false);
  if (state.case !== testCase) Object.assign(camera,{zoom:1,x:0,y:0});
  Object.assign(state, {case:testCase, replay, days, timeline, t:0, progress:0, selectedAgent:null, selectedCell:null});
  $('slider').max = timeline.length - 1;
  buildChapters();
  for (const id of ['play','prev','next','slider','day','step']) $(id).disabled = !replay;
  $('toggleHeatmap').disabled = $('download').disabled = $('exportGif').disabled = !replay; $('saveProblem').disabled = false;
  $('day').replaceChildren(...days.map((d,i) => {const o = element('option',`${d.day}日目`); o.value = i; return o;}));
  const label = `${FORMAT[testCase.format] || '読み込み'} · ${problem.map.width}×${problem.map.height} · ${problem.daySteps.length}日間`;
  $('caseLabel').textContent = `${testCase.seed == null ? '' : `seed ${testCase.seed} · `}${label}`;
  $('mapTab').disabled = false;
  $('mapTab').textContent = replay ? 'リプレイ' : '盤面';
  render(); updateCaseNavigation(); view('map');
}
async function generateCase() {
  await state.navigation.go({kind:'preview',seed:seedInput('previewSeed'),format:$('previewFormat').value,arenaMaps:$('previewArenaMaps').checked});
}
function currentView() {
  const time = state.timeline[state.t];
  const day = state.days[time.d];
  return {...time,day,frame:day.frames[time.s],progress:state.progress,roadStatus:new Map(day.traffics.map(t => [t.pos,t.status]))};
}
function setInspectorTab(name) {
  state.inspectorTab = name;
  for (const button of document.querySelectorAll('[data-inspector]')) {
    const active = button.dataset.inspector === name;
    button.setAttribute('aria-selected',String(active)); button.tabIndex = active ? 0 : -1;
    $(`inspector-${button.dataset.inspector}`).hidden = !active;
  }
  state.navigation?.update();
}
function setInspectorOpen(on) {
  state.inspectorOpen = on; document.body.classList.toggle('inspector-open',on);
  $('toggleInspector').setAttribute('aria-expanded',String(on));
  state.navigation?.update();
}
function resizeBoard() {
  if (!state.case || state.view !== 'map') return;
  const box = $('canvas').parentElement, ratio = Math.min(2,window.devicePixelRatio || 1);
  const width = Math.max(1,Math.floor((box.clientWidth - 16) * ratio)), height = Math.max(1,Math.floor((box.clientHeight - 16) * ratio));
  if ($('canvas').width !== width || $('canvas').height !== height) {
    camera.x *= width / $('canvas').width; camera.y *= height / $('canvas').height;
    $('canvas').width = width; $('canvas').height = height; renderMap();
  }
}
function gifPlan() {
  return planGifFrames(state.days,{day:$('gifScope').value === 'day' ? state.timeline[state.t].d : null,speed:Number($('gifSpeed').value)});
}
function gifHint() {
  const frames = gifPlan(), duration = frames.reduce((sum,f) => sum + f.delay,0) / 1000;
  $('gifHint').textContent = `${frames.length}コマ · 約${duration.toFixed(1)}秒 · 最終フレームを含むループGIF`;
}
function openGifDialog() {
  if (!state.replay) return;
  setPlaying(false);
  if (state.gifUrl) {URL.revokeObjectURL(state.gifUrl); state.gifUrl = null;}
  $('gifReady').hidden = $('gifProgressRow').hidden = $('gifError').hidden = true;
  $('gifOptions').disabled = false; $('makeGif').disabled = false;
  $('gifSource').textContent = `${$('caseLabel').textContent} · 現在 ${state.days[state.timeline[state.t].d].day}日目`;
  gifHint(); $('gifDialog').showModal();
}
async function makeGif(event) {
  event.preventDefault();
  const controller = new AbortController(); state.gifAbort = controller;
  $('gifOptions').disabled = $('makeGif').disabled = true;
  $('gifError').hidden = $('gifReady').hidden = true; $('gifProgressRow').hidden = false;
  $('gifProgress').value = 0; $('gifProgressText').textContent = 'GIFを生成しています…';
  const testCase = state.case, days = state.days, frames = gifPlan(), settings = {...mapSettings(),camera:null};
  const width = Number($('gifWidth').value), map = testCase.problem.map;
  const board = element('canvas'); board.width = width; board.height = Math.ceil(width * (1.5 * map.height + 0.5) / (Math.sqrt(3) * (map.width + 0.5)));
  const output = element('canvas'); output.width = width; output.height = board.height + 92;
  const ctx = output.getContext('2d',{willReadFrequently:true});
  try {
    const blob = await exportGif({width,height:output.height,frames,signal:controller.signal,
      draw:point => {
        const day = days[point.day], frame = day.frames[point.step];
        renderMap(board,{day,frame,s:point.step,roadStatus:new Map(day.traffics.map(t => [t.pos,t.status]))},settings,testCase);
        ctx.fillStyle = '#fff'; ctx.fillRect(0,0,output.width,output.height); ctx.drawImage(board,0,48);
        ctx.fillStyle = '#1f2430'; ctx.textAlign = 'left'; ctx.textBaseline = 'middle'; ctx.font = '600 15px system-ui';
        ctx.fillText(`seed ${testCase.seed ?? 'import'} · ${FORMAT[testCase.format] || 'リプレイ'}`,14,18,width - 28);
        ctx.fillStyle = '#6b7280'; ctx.font = '12px system-ui'; ctx.fillText(`${day.day}日目 / Step ${frame.step} / ${day.steps}`,14,37);
        const y = output.height - 23;
        if (isGame) {ctx.fillStyle=PLAYER_COLORS[settings.player];ctx.fillText(`プレイヤー${settings.player} · 得点3指標で順位判定`,width/2,37,width/2-12);} ctx.fillStyle = '#156082'; ctx.font = '600 13px system-ui';
        [['種類数',frame.score.uniqueBrands],['日別種類',frame.score.dailyBrands],['玉数',frame.score.udon]].forEach(([label,value],i) => ctx.fillText(`${label}  ${value}`,14 + i * width / 3,y,width / 3 - 20));
        return ctx.getImageData(0,0,output.width,output.height);
      },onProgress:(done,total) => {$('gifProgress').max = total; $('gifProgress').value = done; $('gifProgressText').textContent = `生成中 ${done} / ${total}コマ`;}
    });
    controller.signal.throwIfAborted();
    if (state.gifUrl) URL.revokeObjectURL(state.gifUrl);
    state.gifUrl = URL.createObjectURL(blob);
    $('gifPreview').src = $('gifDownload').href = state.gifUrl;
    $('gifDownload').download = `replay-${testCase.seed ?? 'import'}-${$('gifScope').value === 'day' ? `day${days[frames[0].day].day}` : 'all'}.gif`;
    $('gifProgressText').textContent = `完成 · ${(blob.size / 1024 / 1024).toFixed(2)} MiB`;
    $('gifReady').hidden = false;
  } catch (e) {
    if (e.name !== 'AbortError') {$('gifError').textContent = e.message; $('gifError').hidden = false;}
  } finally {
    if (state.gifAbort === controller) {state.gifAbort = null; $('gifOptions').disabled = $('makeGif').disabled = false;}
  }
}
function buildChapters() {
  state.chapters = playbackChapters(state.days);
  $('seekTooltip').hidden = true;
  $('chapterSeek').classList.toggle('disabled',!state.replay);
  $('chapterTrack').replaceChildren(...state.chapters.map(chapter => {
    const segment = element('span',undefined,'chapter-segment');
    segment.style.left = `${chapter.from * 100}%`; segment.style.width = `${(chapter.to - chapter.from) * 100}%`;
    return segment;
  }));
  $('chapterLabels').hidden = !state.replay;
  $('chapterLabels').replaceChildren(...state.chapters.map(chapter => {
    const button = element('button',`${chapter.day}日`);
    button.setAttribute('aria-label',`${chapter.day}日目の先頭へ`);
    button.title = `${chapter.day}日目 · ${state.days.find(day => day.day === chapter.day).steps}ステップ`;
    button.style.left = `${chapter.from * 100}%`; button.style.width = `${(chapter.to - chapter.from) * 100}%`;
    button.onclick = () => {setPlaying(false); setTime(chapter.start);};
    return button;
  }));
}
function renderChapters() {
  const day = state.timeline[state.t].d;
  [...$('chapterTrack').children].forEach((segment,i) => {
    segment.style.setProperty('--filled',`${chapterProgress(state.chapters[i],state.t,state.timeline.length - 1) * 100}%`);
    segment.classList.toggle('current',i === day);
  });
  [...$('chapterLabels').children].forEach((button,i) => button.setAttribute('aria-current',i === day ? 'step' : 'false'));
  const frame = state.days[day].frames[state.timeline[state.t].s];
  $('slider').setAttribute('aria-valuetext',`${state.days[day].day}日目、ステップ${frame.step}`);
}
function seekHover(event) {
  if (!state.replay) return;
  const rect = $('chapterTrack').getBoundingClientRect();
  const fraction = Math.max(0,Math.min(1,(event.clientX - rect.left) / rect.width));
  const point = state.timeline[Math.round(fraction * (state.timeline.length - 1))], day = state.days[point.d];
  $('seekTooltip').textContent = `${day.day}日目 · Step ${day.frames[point.s].step}`;
  $('seekTooltip').style.left = `clamp(65px,${fraction * 100}%,calc(100% - 65px))`;
  $('seekTooltip').hidden = false;
}
function setTime(value,progress = 0) {
  if (!state.timeline.length) return;
  state.t = Math.max(0, Math.min(state.timeline.length - 1, Math.round(value))); state.progress = progress; render();
  if (!state.playing) state.navigation?.update();
}
function setPlaying(on) {state.playing = on; $('play').textContent = on ? '❚❚ 停止' : '▶ 再生'; if (!on) state.navigation?.update();}
let lastTick = 0;
function animate(ts) {
  if (state.playing) {
    const next = advancePlayback(state.t,state.progress,ts - lastTick,Number($('speed').value),state.timeline.length - 1);
    lastTick = ts;
    if (next.frame !== state.t) setTime(next.frame,next.progress);
    else {state.progress = next.progress; if (!state.heatmapShown) renderMap();}
    if (state.t === state.timeline.length - 1) setPlaying(false);
  }
  requestAnimationFrame(animate);
}

function mapSettings() {return {camera,player:state.player,matchReplay:state.matchReplay,showOthers:isGame && $('showOthers').checked,showIds:$('showIds').checked,showRoute:$('showRoute').checked,selectedAgent:state.selectedAgent,selectedCell:state.selectedCell,heatmapMode:state.heatmapShown && state.replay ? $('heatmapMode').value : 'none',heatmapScope:$('heatmapScope').value,heatmapTeams:$('heatmapTeams').value,heatmapDays:state.days};}
function renderMap(canvas = $('canvas'), frameView = currentView(), settings = mapSettings(), testCase = state.case) {
  if (canvas === $('canvas')) $('zoomLevel').textContent = `${Math.round(camera.zoom * 100)}%`;
  const color = isGame ? PLAYER_COLORS[settings.player] : boardPlayerColor(settings.player);
  renderBoard(canvas,testCase.problem,frameView,{...settings,agentColor:() => color,
    routeOverlay:(ctx,center,size) => drawOtherPlayerRoutes(ctx,center,size,settings,frameView.day.day),
    overlay:(ctx,center,size) => drawOtherPlayers(ctx,center,size,{...settings,fuelLimits:testCase.problem.fuelLimits},frameView.day.day,frameView.s,frameView.progress ?? 0)});
}
function renderBrands() {
  const {brands,daily} = currentView(), all = [...new Set(state.case.problem.spots.map(s => s.brand))].sort((a,b) => a - b);
  for (const [id,count,got] of [['brands','brandCount',brands],['dayBrands','dayBrandCount',daily]]) {
    $(count).textContent = `${got.size}/${all.length}`;
    $(id).replaceChildren(...all.map(b => {
      const chip = element('span',b,`chip${got.has(b) ? ' on' : ''}`), color = brandColor(b);
      chip.style.borderColor = color; chip.style.color = got.has(b) ? '#fff' : color; chip.style.background = got.has(b) ? color : '#fff';
      chip.title = `系列 ${b} · ${got.has(b) ? '取得済み' : '未取得'}`; return chip;
    }));
  }
}
function failureText(f) {return `${f.kind === 'timeout' ? 'TLE' : f.kind === 'process' ? 'RE' : 'WA'}: ${f.message}${f.agent == null ? '' : `（車${f.agent} / step ${f.step}）`}`;}
function render() {
  if (!state.case) return;
  const {d,s,day,frame,answerMs} = currentView(), cfg = state.case.problem;
  $('slider').value = state.t; $('day').value = d; $('step').value = frame.step; $('step').max = day.steps;
  $('tpos').textContent = state.replay ? `${day.day}日目 / Step ${frame.step} / ${day.steps}` : '初期配置';
  renderChapters(); renderHeatmapControls();
  $('prev').disabled = !state.replay || state.t === 0; $('next').disabled = !state.replay || state.t === state.timeline.length - 1;
  const stats = [['試合の種類数',frame.score.uniqueBrands],['日別種類数の合計',frame.score.dailyBrands],['獲得玉数',frame.score.udon],['有効回答時間',`${answerMs || 0} ms`],['燃料上限',cfg.fuelLimits],['交通量の基準',`${cfg.busyThreshold} / ${cfg.jammedThreshold}`]];
  const tbody = element('tbody');
  stats.forEach(([label,value]) => {const row = element('tr'); row.append(element('td',label),element('td',value)); tbody.append(row);});
  $('stats').replaceChildren(tbody); renderBrands(); renderMap();
  const head = element('thead'), hr = element('tr'); ['車','タイプ','位置','燃料','予約'].forEach(label => hr.append(element('th',label))); head.append(hr);
  const agents = element('tbody');
  frame.agents.forEach((a,i) => {
    const row = element('tr',undefined,state.selectedAgent === i ? 'selected' : ''), pending = frame.pending[i];
    const action = pending ? `${pending.destination == null ? '待機' : `→ ${pending.destination}`} (${pending.until - frame.step})` : '—';
    [i,a.kind === 0 ? '巡回車' : '補給車',a.pos,a.kind === 0 ? `${a.fuel}/${cfg.fuelLimits}` : '∞',action].forEach(v => row.append(element('td',v)));
    row.tabIndex = 0; row.setAttribute('aria-label',`車${i}を選択`);
    const choose = () => {state.selectedAgent = state.selectedAgent === i ? null : i; render(); state.navigation?.update();};
    row.onclick = choose; row.onkeydown = e => {if (e.key === 'Enter' || e.key === ' ') {e.preventDefault(); choose();}};
    agents.append(row);
  });
  $('agents').replaceChildren(head,agents);
  const messages = frame.events.map(e => {
    if (e.kind === 'move') return `車${e.agent}: ${e.from} → ${e.to}`;
    if (e.kind === 'collect') return `車${e.agent}: 系列${e.brand}のうどんを取得（店舗${e.spot}）`;
    if (e.kind === 'refill') return `車${e.agent}: 燃料を${e.amount}補給`;
    if (e.kind === 'fuel') return `車${e.agent}: 燃料を${e.amount}消費`;
    return e.kind;
  });
  $('events').replaceChildren(...(messages.length ? messages : [s === 0 ? '行動予約・当日の初期状態' : 'このステップの出来事はありません。']).map(m => element('p',m)));
  const failures = [state.replay?.selectionFailure,day.failure].filter(Boolean);
  $('dayFailure').hidden = !failures.length; $('dayFailure').textContent = failures.map(failureText).join('\n');
  renderCellInfo();
  if (isGame) renderStandings(d,s);
}
function renderHeatmapControls() {
  const settings = mapSettings(), on = settings.heatmapMode !== 'none';
  document.body.classList.toggle('heatmap-mode',on);
  $('heatmapControls').hidden = !on; $('toggleHeatmap').setAttribute('aria-pressed',String(on));
  $('toggleHeatmap').classList.toggle('active',on); $('heatmapTeamsField').hidden = !state.matchReplay;
  if (!on) return;
  const {day} = currentView(), cfg = state.case.problem, counts = residenceHeatmap(day,cfg.map.width * cfg.map.height,settings);
  const combined = settings.heatmapMode === 'both', supply = settings.heatmapMode === 'supply';
  const kind = combined ? '合計（巡回車＋補給車）' : supply ? '補給車' : '巡回車';
  const max = combined ? counts.maxTotal : supply ? counts.maxSupply : counts.maxPatrol;
  const scale = $('heatmapScale'); scale.classList.toggle('supply',supply);
  scale.replaceChildren(element('span',`${kind} 0`),element('span',undefined,'heatmap-gradient'),element('span',`${max}ステップ`));
  $('heatmapDescription').textContent = `${settings.heatmapScope === 'all' ? '全日合計' : `${day.day}日目`} · 対数スケール · 延べ滞在時間（移動中は出発セル）`;
}
function renderCellInfo() {
  if (state.selectedCell == null) {$('cellInfo').textContent = state.heatmapShown ? 'セルをクリックすると合計と車種別の滞在ステップ数を確認できます。' : 'セルをクリックすると詳細を表示します。'; return;}
  const pos = state.selectedCell, cfg = state.case.problem, {frame,day,roadStatus} = currentView();
  const terrain = cfg.map.cells[Math.floor(pos / cfg.map.width)][pos % cfg.map.width];
  const info = [`セル ${pos}`,['平地','道路','山地','池'][terrain]];
  if (terrain === 1) info.push(['順調','混雑','渋滞'][roadStatus.get(pos) ?? 0]);
  cfg.spots.forEach((spot,i) => {if (spot.pos === pos) info.push(`店舗${i} · 系列${spot.brand} · 在庫${frame.stocks[i]}/${spot.stocks}`);});
  frame.agents.forEach((a,i) => {if (a.pos === pos) info.push(`車${i}`);});
  if (state.heatmapShown) {const counts = residenceHeatmap(day,cfg.map.width * cfg.map.height,mapSettings()); info.push(`合計 ${counts.total[pos]}ステップ`,`巡回車 ${counts.patrol[pos]}ステップ`,`補給車 ${counts.supply[pos]}ステップ`);}
  $('cellInfo').textContent = $('cellInfo').title = info.join(' / ');
}

function runStatus(m) {return `${m.importedFrom ? '取込 · ' : ''}${STATUS[m.status] || m.status}${m.verdict ? ` · ${m.verdict}` : ''}`;}
function executionText(m) {return `同時seed ${m.execution?.jobs ?? m.config.jobs ?? '自動'} · ソルバ内並列${(m.config.solverParallel || m.config.opponents?.some(p=>p.solverParallel)) ? 'あり' : 'なし'}${m.execution ? ` · CPU ${m.execution.availableCpus}` : ''} · 非公式練習場のマップ${m.config.arenaMaps === false ? 'なし' : 'あり（1/4）'}`;}
function busy(manifest) {return ['building','running'].includes(manifest.status);}
function runDate(id) {
  try {
    id = state.runs.find(m => m.id === id)?.importedFrom || state.comparison?.runs.find(m => m.id === id)?.importedFrom || id;
    const date = new Date(Number(BigInt(id.split('-')[0]) / 1000000n));
    return new Intl.DateTimeFormat('ja-JP',{month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',second:'2-digit',hour12:false}).format(date);
  } catch {return id;}
}
function runRange(m) {return m.config.seed != null ? `seed ${m.config.seed}` : `seed 0〜${m.total - 1}`;}
function resultFilters() {
  return {format:$('filterFormat').value,status:$('filterStatus').value,search:$('filterSeed').value,sort:$('sortCases').value};
}
function applyFilters(filters = {}) {
  $('filterFormat').value = filters.format || ''; $('filterStatus').value = filters.status || '';
  $('filterSeed').value = filters.search || ''; $('sortCases').value = filters.sort || 'seed';
}
function comparisonOptions() {
  return {baseline:state.comparison?.runs[Number($('compareBaseline').value)]?.id,metric:$('compareMetric').value,format:$('compareFormat').value,filter:$('compareFilter').value};
}
function captureRoute(route) {
  const saved = {...route,scroll:window.scrollY,sidebarScroll:$('history').scrollTop};
  if (route.kind === 'run' && state.runView) Object.assign(saved,{view:state.view,seed:state.openedSeed,filters:resultFilters()});
  if (route.kind === 'compare') Object.assign(saved,comparisonOptions());
  if (state.case && ['run','preview','import'].includes(route.kind)) Object.assign(saved,{
    ...(isGame ? {player:state.player,showOthers:$('showOthers').checked} : {}),frame:state.t,selectedAgent:state.selectedAgent,selectedCell:state.selectedCell,
    heatmapMode:mapSettings().heatmapMode,heatmapScope:$('heatmapScope').value,heatmapTeams:$('heatmapTeams').value,
    showIds:$('showIds').checked,showRoute:$('showRoute').checked,speed:$('speed').value,inspectorTab:state.inspectorTab,inspectorOpen:state.inspectorOpen
  });
  return saved;
}
function openRun(id,seed = null) {
  return state.navigation.go({kind:'run',id,view:seed == null ? 'results' : 'map',seed,frame:0});
}
function openReplay(id,seed) {
  return state.navigation.go({kind:'run',id,view:'map',seed,frame:0,filters:resultFilters()});
}
function openComparison() {
  return state.navigation.go({kind:'compare',ids:[...state.compareIds],...comparisonOptions()});
}
function switchView(name) {
  const route = captureRoute(state.navigation.route);
  return state.navigation.go({...route,view:name,scroll:0});
}
async function renderRoute(route,current) {
  document.body.classList.remove('replay-mode');
  state.runRequest++; state.replayRequest++; state.compareMode = false;
  for (const dialog of document.querySelectorAll('dialog[open]')) dialog.close();
  setPlaying(false); error('');
  if (route.kind === 'run') {
    try {await loadRun(route.id,route.seed);} catch (e) {
      if (e.status !== 404 || e.path !== `/api/runs/${route.id}` || !current()) throw e;
      forgetRun(route.id);
      await refreshHistory();
      if (current()) await state.navigation.go(route,{replace:true});
      return;
    }
    if (!current()) return;
    applyFilters(route.filters); renderResults(); updateCaseNavigation();
    view(route.view === 'map' && !state.case ? 'results' : route.view);
  } else if (route.kind === 'compare') {
    state.compareIds = [...route.ids];
    $('compareMetric').value = route.metric || 'uniqueBrands'; $('compareFormat').value = route.format || ''; $('compareFilter').value = route.filter || 'all';
    await loadComparison(route.ids,route.baseline);
  } else if (route.kind === 'preview') {
    standalone('問題のプレビュー','読み込み中','盤面を生成しています…');
    for (const key of ['results','map','logs','compare']) $(`${key}View`).hidden = true;
    $('previewSeed').value = route.seed; $('previewFormat').value = route.format || 'auto'; $('previewArenaMaps').checked = route.arenaMaps !== false;
    const testCase = await api(`/api/generate?seed=${route.seed}&format=${route.format || 'auto'}&arenaMaps=${route.arenaMaps !== false}`);
    if (!current()) return;
    standalone('問題のプレビュー','未実行','生成した盤面です。実行履歴には保存されていません。');
    loadCase(testCase);
  } else if (route.kind === 'import') {
    const imported = state.imports.get(route.id);
    if (!imported) throw new Error('読み込んだファイルはこのタブの再読み込み前のものです。「JSONファイルを開く」から再度開いてください。');
    const {name,data} = imported;
    standalone(name,'ファイルから読み込み','読み込んだファイルの内容を表示しています。');
    if ([1,2].includes(data.schemaVersion)) loadCase(data.case,data);
    else if (data.problem) loadCase(data); else loadCase({problem:data,seed:null,format:null});
  } else {
    state.selectedRunId = null; state.runView = null; clearCase(); state.view = 'home';
    for (const id of ['recordHeader','viewTabs','resultsView','mapView','logsView','compareView']) $(id).hidden = true;
    $('emptyView').hidden = false;
  }
  if (!current()) return;
  if (state.case && route.kind !== 'compare') {
    if (isGame && state.matchReplay) {
      const selectedView=state.view;
      loadCase(state.case,state.matchReplay,Number.isInteger(route.player) && route.player>=0 && route.player<5 ? route.player : 0);
      view(selectedView);
      $('showOthers').checked=route.showOthers??true;
      await loadPlayerLog();
      if (!current()) return;
    }
    state.selectedAgent = route.selectedAgent ?? null; state.selectedCell = route.selectedCell ?? null;
    state.heatmapShown = ['both','patrol','supply'].includes(route.heatmapMode);
    $('heatmapMode').value = state.heatmapShown ? route.heatmapMode : 'both'; $('heatmapScope').value = route.heatmapScope || 'day'; $('heatmapTeams').value = route.heatmapTeams || 'selected';
    $('showIds').checked = route.showIds ?? false; $('showRoute').checked = route.showRoute ?? true; $('speed').value = route.speed || '1';
    setInspectorTab(route.inspectorTab || state.inspectorTab); setInspectorOpen(route.inspectorOpen ?? state.inspectorOpen);
    setTime(route.frame || 0);
  }
  renderHistory(); $('history').scrollTop = route.sidebarScroll ?? $('history').scrollTop;
}
function chooseComparison() {
  state.compareMode = true;
  if (!state.compareIds.length && state.selectedRunId) state.compareIds = [state.selectedRunId];
  renderHistory();
}
function openDeleteDialog(manifest) {
  if (busy(manifest)) return;
  state.deleteTarget = manifest.id;
  $('deleteTarget').textContent = `${manifest.config.label || runName(manifest)}\n${runDate(manifest.id)} · ${runRange(manifest)} · ${manifest.completed}件の結果`;
  $('deleteError').hidden = true; $('deleteDialog').showModal();
}
function updateExportButton() {
  const m = state.runView?.manifest;
  $('exportRun').hidden = !m;
  $('exportRun').disabled = !m || busy(m) || state.exporting.has(m.id);
  $('exportRun').textContent = m && state.exporting.has(m.id) ? 'ZIPを準備中…' : '履歴をエクスポート';
}
async function exportHistory(id) {
  if (state.exporting.has(id)) return;
  state.exporting.add(id); renderHistory(); updateExportButton();
  try {
    const response = await fetch(`/api/runs/${id}/export`);
    if (!response.ok) {const error = await response.json(); throw new Error(error.error || 'エクスポートに失敗しました。');}
    const url = URL.createObjectURL(await response.blob()), link = element('a');
    link.href = url; link.download = `run-${id}.procon37.zip`; link.click();
    setTimeout(() => URL.revokeObjectURL(url),60000);
  } finally {state.exporting.delete(id); renderHistory(); updateExportButton();}
}
async function importHistory() {
  const file = $('historyFile').files[0];
  if (!file) throw new Error('履歴ZIPを選んでください。');
  if (file.size > 256 * 1024 * 1024) throw new Error('ZIPは256 MiB以下にしてください。');
  state.importing = true; $('importProgress').hidden = false;
  const controls = [...$('importDialog').querySelectorAll('input,button[data-close]')];
  controls.forEach(control => {control.disabled = true;});
  try {
    const query = new URLSearchParams({label:$('importLabel').value.trim()});
    const response = await fetch(`/api/runs/import?${query}`,{method:'POST',headers:{'Content-Type':'application/zip'},body:file});
    const raw = await response.text(); let result;
    try {result = JSON.parse(raw);} catch {throw new Error(raw || 'インポートに失敗しました。');}
    if (!response.ok) throw new Error(result.error || 'インポートに失敗しました。');
    rememberRun(result); await refreshHistory(); $('importDialog').close(); await openRun(result.id);
  } finally {state.importing = false; $('importProgress').hidden = true; controls.forEach(control => {control.disabled = false;});}
}
function forgetRun(id) {
  state.deletedIds.add(id);
  state.runs = state.runs.filter(m => m.id !== id);
  state.compareIds = state.compareIds.filter(value => value !== id);
  try {sessionStorage.setItem(storagePrefix+'.deleted-runs',JSON.stringify([...state.deletedIds]));} catch { /* Still handled in this page. */ }
}
async function deleteHistory() {
  const id = state.deleteTarget;
  if (!id) return;
  try {await api(`/api/runs/${id}`,undefined,'DELETE');} catch (e) {if (e.status !== 404) throw e;}
  forgetRun(id);
  if (state.activeId === id) {state.activeId = null; $('activePanel').hidden = true;}
  $('deleteDialog').close(); state.deleteTarget = null;
  const route = captureRoute(state.navigation.route), next = withoutDeletedRuns(route,state.deletedIds,state.runs[0]?.id);
  if (next !== route) await state.navigation.go(next,{replace:true});
  else renderHistory();
}

function totalsTable(cases,compact = false) {
  const table = element('table',undefined,`data-table totals-table${compact ? ' compact-totals' : ''}`), head = element('thead'), body = element('tbody');
  const heading = element('tr');
  for (const text of ['形式',...(!compact ? ['件数'] : []),'種類数','日別種類','玉数']) heading.append(element('th',text));
  head.append(heading);
  for (const [format,label] of [['','全体'],...Object.entries(FORMAT)]) {
    const selected = cases.filter(c => !format || c.format === format), totals = scoreTotals(selected), row = element('tr',undefined,format ? '' : 'total-row');
    for (const value of [label,...(!compact ? [selected.length] : []),...SCORE_KEYS.map(key => totals[key].toLocaleString('ja-JP'))]) row.append(element('td',value));
    body.append(row);
  }
  table.append(head,body); return table;
}
function totalTriplet(cases) {
  const totals = scoreTotals(cases), group = element('span',undefined,'history-totals');
  for (const [key,label] of [['uniqueBrands','種類数'],['dailyBrands','日別種類'],['udon','玉数']]) {
    const item = element('span'); item.append(element('small',label),element('b',totals[key].toLocaleString('ja-JP'))); group.append(item);
  }
  return group;
}

function rememberRun(m) {
  if (state.deletedIds.has(m.id)) return;
  const i = state.runs.findIndex(r => r.id === m.id);
  if (i < 0) state.runs.unshift(m); else state.runs[i] = m;
}
function renderHistory() {
  $('history').replaceChildren(...state.runs.map(m => {
    const item = element('button',undefined,`history-item${m.id === state.selectedRunId ? ' selected' : ''}`);
    item.setAttribute('aria-current',m.id === state.selectedRunId ? 'true' : 'false');
    item.title = m.id;
    const top = element('span',undefined,'history-line');
    top.append(element('span',runDate(m.id),'history-date'),element('span',runStatus(m)));
    const bottom = element('span',undefined,'history-line');
    const issues = m.cases.filter(c => c.status !== 'AC').length;
    const detail = m.verdict === 'CE' ? 'CE' : m.error ? '実行中断' : issues ? ['WA','TLE','RE'].map(v => {const n = m.cases.filter(c => c.status === v).length; return n ? `${v} ${n}` : null;}).filter(Boolean).join(' · ') : busy(m) ? '実行中' : m.completed ? 'AC' : '結果なし';
    bottom.append(element('span',`${m.completed}/${m.total}件`),element('span',detail,(issues || m.error) ? 'warn' : ''));
    item.append(top,element('strong',m.config.label || `${FORMAT[m.config.format] || '形式自動決定'} · ${runRange(m)}`),bottom);
    if (isGame) item.append(element('span',`対象の平均順位 ${averageRankText(matchStats(m.cases).averageRank)}`,'history-average'));
    item.append(totalTriplet(m.cases));
    item.onclick = guard(() => openRun(m.id));
    const wrapper = element('div',undefined,`history-entry${state.compareMode ? ' choosing' : ''}`), choice = element('label',undefined,'compare-choice'), check = element('input');
    check.type = 'checkbox'; check.checked = state.compareIds.includes(m.id); check.disabled = !check.checked && state.compareIds.length >= 4;
    check.setAttribute('aria-label',`${runDate(m.id)} ${m.config.label || runRange(m)} を比較に選択`);
    check.onchange = () => {state.compareIds = check.checked ? [...state.compareIds,m.id] : state.compareIds.filter(id => id !== m.id); renderHistory();};
    choice.append(check,document.createTextNode('比較に選択')); wrapper.append(item);
    if (state.compareMode) wrapper.append(choice);
    const footer = element('div',undefined,'history-footer'), breakdown = element('details',undefined,'history-breakdown');
    breakdown.append(element('summary','形式別の合計'),totalsTable(m.cases,true));
    const remove = element('button','削除','quiet history-delete'); remove.disabled = busy(m);
    remove.setAttribute('aria-label',`${runDate(m.id)} ${m.config.label || runRange(m)} の履歴を削除`);
    remove.title = busy(m) ? '実行中の履歴は削除できません' : 'この実行履歴を削除';
    const exportButton = element('button',state.exporting.has(m.id) ? '準備中…' : 'ZIP出力','quiet history-export');
    exportButton.disabled = busy(m) || state.exporting.has(m.id);
    exportButton.setAttribute('aria-label',`${runDate(m.id)} ${m.config.label || runRange(m)} の履歴をエクスポート`);
    exportButton.title = busy(m) ? '実行中の履歴はエクスポートできません' : '問題・リプレイ・ログを含む履歴ZIPを保存';
    exportButton.onclick = guard(() => exportHistory(m.id));
    remove.onclick = () => openDeleteDialog(m); footer.append(breakdown,exportButton,remove); wrapper.append(footer);
    return wrapper;
  }));
  $('chooseComparison').hidden = state.compareMode; $('compareControls').hidden = !state.compareMode;
  $('compareSelected').disabled = state.compareIds.length < 2; $('compareCount').textContent = `${state.compareIds.length} / 4件を選択`;
  if (!state.runs.length) $('history').append(element('p','実行履歴はまだありません。','note'));
}
function updateProgress(m) {
  $('activePanel').hidden = !busy(m);
  $('progress').max = m.total; $('progress').value = m.completed;
  $('progressText').textContent = `${runStatus(m)} · ${m.completed} / ${m.total}`;
  $('run').disabled = busy(m) || state.submitting;
  $('runBusyNote').hidden = !busy(m); $('cancel').disabled = !busy(m);
}
function clearCase() {
  document.body.classList.remove('replay-mode','heatmap-mode');
  setPlaying(false); state.replayRequest++;
  state.case = null; state.replay = null; state.openedSeed = null; state.timeline = [];
  $('exportRun').hidden = true;
  $('mapTab').disabled = true; $('mapTab').textContent = 'リプレイ';
  $('solverLogTitle').textContent = 'ソルバログ（stderr）';
  $('solverLog').textContent = '結果一覧からケースを開くと表示されます。';
}
function resetFilters() {
  $('filterFormat').value = ''; $('filterStatus').value = ''; $('filterSeed').value = ''; $('sortCases').value = 'seed';
}
function showRecordHeader() {
  $('compareView').hidden = true; $('emptyView').hidden = true; $('recordHeader').hidden = false; $('viewTabs').hidden = false;
}
async function loadRun(id, seed = null) {
  const token = ++state.runRequest;
  state.selectedRunId = id; state.runView = null; clearCase(); resetFilters(); renderHistory();
  showRecordHeader();
  $('recordSource').textContent = '保存された実行'; $('recordTitle').textContent = '記録を読み込んでいます…';
  $('recordMeta').textContent = ''; $('rerun').disabled = true;
  for (const key of ['results','map','logs','compare']) $(`${key}View`).hidden = true;
  try {
    const result = await api(`/api/runs/${id}`);
    if (token !== state.runRequest) return;
    renderRun(result); view('results');
    if (seed != null && result.manifest.cases.some(c => c.seed === seed && !c.error)) await loadReplay(id,seed);
  } catch (e) {if (token === state.runRequest) throw e;}
}
function renderRun(result) {
  const m = result.manifest;
  if (m.id !== state.selectedRunId) return;
  state.runView = result; rememberRun(m);
  showRecordHeader();
  $('recordSource').textContent = `${m.importedFrom ? 'インポートした実行' : '保存された実行'} · ${runDate(m.id)}`;
  $('recordTitle').textContent = m.config.label || `${FORMAT[m.config.format] || '形式自動決定'} / ${runRange(m)}`;
  $('recordMeta').textContent = `${FORMAT[m.config.format] || '形式自動決定'} · ${runRange(m)} · ${executionText(m)} · ${(m.config.build || m.config.opponents?.some(p=>p.build)) ? 'ビルドあり' : 'ビルド省略'}`;
  $('recordStatus').textContent = runStatus(m);
  $('recordStatus').className = `badge${m.verdict && m.verdict !== 'AC' ? ' warning' : m.status === 'completed' ? '' : ' neutral'}`;
  $('rerun').hidden = false; $('rerun').disabled = false;
  updateExportButton();
  $('resultsTab').disabled = false; $('logsTab').disabled = false;
  $('recordConfig').textContent = JSON.stringify(m.config,null,2);
  $('recordConfig').title = `実行ID: ${m.id}`;
  $('buildLog').textContent = result.buildLog || 'ビルドログはありません。';
  const values = [['完了 / 全体',`${m.completed} / ${m.total}`], ...['AC','WA','TLE','RE'].map(v => [v,m.cases.filter(c => c.status === v).length]), ['CE',m.verdict === 'CE' ? 1 : 0]];
  $('runMetrics').replaceChildren(...values.map(([label,value]) => {const card = element('div',undefined,'card metric'); card.append(element('span',label),element('strong',value)); return card;}));
  $('runTotals').replaceChildren(totalsTable(m.cases));
  if (isGame) renderMatchSummary($('matchTotals'),m.cases,playerNames(),element);
  $('totalsNote').textContent = `完了した${m.completed}件の合計${m.cases.some(c => c.status !== 'AC') ? ' · AC以外も含む' : ''}`;
  renderResults(); renderHistory(); updateCaseNavigation();
}
function filteredCases() {
  const rows = (state.runView?.manifest.cases || []).filter(c =>
    (!$('filterFormat').value || c.format === $('filterFormat').value) &&
    (!$('filterStatus').value || c.status === $('filterStatus').value) &&
    c.seed.includes($('filterSeed').value.trim()));
  const compareSeed = (a,b) => BigInt(a.seed) < BigInt(b.seed) ? -1 : BigInt(a.seed) > BigInt(b.seed) ? 1 : 0;
  return rows.sort((a,b) => {
    if (isGame && $('sortCases').value === 'rank') return Number(b.status === 'AC') - Number(a.status === 'AC') || (a.players?.[0]?.rank ?? 6) - (b.players?.[0]?.rank ?? 6) || compareSeed(a,b);
    if ($('sortCases').value === 'score') {
      // Official priority, with valid runs before fallback/error cases.
      const validity = Number(b.status === 'AC') - Number(a.status === 'AC');
      const score = b.score.uniqueBrands - a.score.uniqueBrands || b.score.dailyBrands - a.score.dailyBrands || b.score.udon - a.score.udon || a.answerTimeMs - b.answerTimeMs;
      return validity || score || compareSeed(a,b);
    }
    if ($('sortCases').value === 'issues') {const rank = {TLE:0,RE:1,WA:2,AC:3}; return rank[a.status] - rank[b.status] || compareSeed(a,b);}
    return compareSeed(a,b);
  });
}
function renderResults() {
  if (!state.runView) return;
  const m = state.runView.manifest, cases = filteredCases();
  const sums = Object.keys(FORMAT).map(f => {
    const rows = cases.filter(c => c.format === f && c.status === 'AC');
    return rows.length ? `${FORMAT[f]} ${(rows.reduce((sum,c) => sum + c.score.uniqueBrands,0) / rows.length).toFixed(2)}` : null;
  }).filter(Boolean);
  $('resultSummary').textContent = `表示 ${cases.length} / ${m.completed}件${sums.length ? ` · ACケースの平均種類数: ${sums.join(' / ')}` : ''}${m.error ? ` · ${m.error}` : ''}`;
  const body = element('tbody');
  cases.forEach(c => {
    const row = element('tr',undefined,c.seed === state.openedSeed ? 'selected' : '');
    [c.seed,FORMAT[c.format]].forEach(v => row.append(element('td',v)));
    if (isGame) {const rank=element('td');rank.append(caseRankBadge(element,c));row.append(rank);}
    [c.score.uniqueBrands,c.score.dailyBrands,c.score.udon,`${c.answerTimeMs} ms`].forEach(v => row.append(element('td',v)));
    const status = element('td',c.status,c.status === 'AC' ? 'good' : 'warn');
    status.title = c.error || ''; row.append(status);
    const cell = element('td'), button = element('button',!!c.error ? '理由' : '再生');
    button.setAttribute('aria-label',!!c.error ? `seed ${c.seed}のエラー` : `seed ${c.seed}を再生`);
    button.onclick = !!c.error ? () => error(c.error || '実行エラー') : guard(() => openReplay(m.id,c.seed));
    cell.append(button); row.append(cell); body.append(row);
  });
  $('results').tBodies[0].replaceWith(body);
  $('noCases').hidden = cases.length > 0;
  $('noCases').textContent = m.completed ? '条件に一致するケースがありません。絞り込みを変更してください。' : busy(m) ? 'ケースが完了すると、ここに結果が追加されます。' : '保存されたケースはありません。';
}
function updateCaseNavigation() {
  const cases = filteredCases().filter(c => !c.error);
  const index = cases.findIndex(c => c.seed === state.openedSeed);
  $('previousCase').disabled = index <= 0;
  $('nextCase').disabled = index < 0 || index >= cases.length - 1;
  $('previousCase').hidden = $('nextCase').hidden = $('backToResults').hidden = !state.runView;
  if (state.runView && state.openedSeed != null) $('caseOrigin').textContent = `保存済みリプレイ · ${runDate(state.selectedRunId)} · ${index < 0 ? '現在の絞り込みの対象外' : `一覧の表示順 ${index + 1} / ${cases.length}件`}`;
}
async function adjacentCase(offset) {
  const cases = filteredCases().filter(c => !c.error), i = cases.findIndex(c => c.seed === state.openedSeed);
  if (i >= 0 && cases[i + offset]) await openReplay(state.selectedRunId,cases[i + offset].seed);
}
async function loadReplay(id,seed) {
  const token = ++state.replayRequest;
  setPlaying(false);
  const replay = await api(`/api/runs/${id}/${seed}/replay`);
  if (token !== state.replayRequest || id !== state.selectedRunId) return;
  state.openedSeed = seed; loadCase(replay.case,replay); renderResults();
  $('solverLogTitle').textContent = `seed ${seed} のソルバログ（stderr）`;
  $('solverLog').textContent = 'ログを読み込んでいます…';
  const log = await fetch(`/api/runs/${id}/${seed}/stderr?player=${state.player}`);
  const text = log.ok ? (await log.text()) || 'stderrへの出力はありません。' : 'stderrログを読み込めませんでした。';
  if (token === state.replayRequest && id === state.selectedRunId) $('solverLog').textContent = text;
}
function standalone(title,status,description) {
  state.runRequest++; state.selectedRunId = null; state.runView = null; clearCase(); renderHistory(); showRecordHeader();
  $('recordSource').textContent = '盤面の閲覧'; $('recordTitle').textContent = title; $('recordMeta').textContent = description;
  $('recordStatus').textContent = status; $('recordStatus').className = 'badge neutral'; $('rerun').hidden = true;
  $('resultsTab').disabled = true; $('logsTab').disabled = true; $('caseOrigin').textContent = description;
}
async function refreshHistory() {
  const [runs,active] = await Promise.all([api('/api/runs'),api('/api/active')]);
  state.runs = runs.filter(m => !state.deletedIds.has(m.id)); renderHistory();
  if (active) {state.activeId = busy(active.manifest) ? active.manifest.id : null; updateProgress(active.manifest);}
}
function fillConfig(cfg) {
  if (isGame) {fillOpponents($,cfg);$('targetName').value=cfg.targetName || '対象ソルバ';}
  $('buildEnabled').checked = cfg.build !== null; $('buildEnabled').onchange();
  $('buildConfig').value = JSON.stringify(cfg.build || {program:'cargo',args:['build','--release','-p','solver'],cwd:cfg.solver.cwd,env:{}},null,2);
  $('solverConfig').value = JSON.stringify(cfg.solver,null,2); $('jobs').value = cfg.jobs ?? '';
  $('solverParallel').value = String(cfg.solverParallel ?? false); $('runLabel').value = cfg.label || '';
  $('timeout').value = cfg.timeLimitMs ?? ''; $('format').value = cfg.format || 'auto';
  $('arenaMaps').checked = cfg.arenaMaps !== false;
  $('mode').value = cfg.seed != null ? 'single' : 'batch'; $('seed').value = cfg.seed ?? '0'; $('count').value = cfg.count ?? 60; $('mode').onchange();
}
function openRunDialog(reuse = false) {
  if (reuse && state.runView) fillConfig(rerunConfig(state.runView.manifest,state.localConfig));
  $('runDialogTitle').textContent = reuse ? '設定を引き継いで実行' : '新しい実行';
  $('runDialogHint').textContent = reuse ? `${runDate(state.selectedRunId)} の設定をコピーしました。結果は新しい実行として保存されます。` : '実行が終わると、結果とリプレイが自動保存されます。';
  if (reuse && state.runView?.manifest.importedFrom) $('runDialogHint').textContent += ' 実行コマンドはこの端末の既定値を使います。';
  $('runError').hidden = true; $('runDialog').showModal();
}
async function startRun() {
  const single = $('mode').value === 'single';
  const config = {count:single ? null : integer('count',1,10000), seed:single ? seedInput() : null,
    format:$('format').value === 'auto' ? null : $('format').value, jobs:$('jobs').value ? integer('jobs',1,256) : null,
    solverParallel:$('solverParallel').value === 'true', arenaMaps:$('arenaMaps').checked, label:$('runLabel').value.trim(),
    build:$('buildEnabled').checked ? JSON.parse($('buildConfig').value) : null,
    solver:JSON.parse($('solverConfig').value), timeLimitMs:$('timeout').value ? integer('timeout',1,3600000) : null};
  if (isGame) {config.mode='game';config.targetName=$('targetName').value.trim();config.opponents=readOpponents($);}
  const result = await api('/api/runs',config);
  state.activeId = result.manifest.id; updateProgress(result.manifest);
  await openRun(result.manifest.id);
  await refreshHistory();
}
async function poll() {
  if (!state.activeId || state.pollBusy) return;
  state.pollBusy = true;
  try {
    const result = await api(`/api/runs/${state.activeId}`), m = result.manifest;
    updateProgress(m); rememberRun(m);
    if (state.selectedRunId === m.id) {
      const previous = state.runView?.manifest;
      if (!previous || previous.status !== m.status || previous.completed !== m.completed) renderRun(result);
      else {$('buildLog').textContent = result.buildLog || 'ビルドログはありません。'; state.runView = result;}
    } else renderHistory();
    if (!busy(m)) {state.activeId = null; await refreshHistory();}
    $('connection').textContent = 'ローカル実行';
  } catch (e) {$('connection').textContent = '接続を確認'; error(e.message);} finally {state.pollBusy = false;}
}
function parallelHint() {
  const total = $('mode').value === 'single' ? 1 : Math.max(1,Number($('count').value) || 60);
  const parallel = $('solverParallel').value === 'true' || (isGame && [1,2,3,4].some(id=>$(`opponent-${id}-parallel`).value==='true')), override = Number($('jobs').value);
  const jobs = Math.min(total,override || (parallel ? 1 : Math.max(1,Math.floor(state.availableCpus/(isGame?5:1)))));
  $('parallelHint').textContent = `利用可能CPU ${state.availableCpus} · 同時に実行するseed ${jobs}件${override ? '（手動指定）' : '（自動）'}。${parallel ? 'ソルバのスレッド数はソルバ側で設定します。' : '各ソルバに1CPU相当を割り当てる想定です。'}${$('mode').value === 'batch' ? '日程と制限時間から長いケースを先に開始します。' : ''}`;
}
function runName(m) {return m.config.label || m.config.solver.program.split('/').pop();}
function caseValue(c,metric) {return metric === 'answerTimeMs' ? c.answerTimeMs : c.score[metric];}
function difference(value,lowerIsBetter = false) {
  const span = element('span',`Δ ${value > 0 ? '+' : ''}${Number.isInteger(value) ? value : value.toFixed(2)}`,`delta ${value === 0 ? '' : (lowerIsBetter ? value < 0 : value > 0) ? 'good' : 'bad'}`);
  return span;
}
async function loadComparison(ids,baselineId) {
  if (ids.length < 2) throw new Error('比較する履歴を2〜4件選んでください。');
  const token = ++state.runRequest;
  state.selectedRunId = null; state.runView = null; state.comparison = null; clearCase(); state.view = 'compare'; renderHistory();
  for (const key of ['emptyView','recordHeader','viewTabs','resultsView','mapView','logsView']) $(key).hidden = true;
  $('compareView').hidden = false; $('compareDescription').textContent = '保存された問題を照合しています…';
  $('compareCards').replaceChildren(); $('comparisonTable').replaceChildren(); $('compareSummary').textContent = ''; $('compareEmpty').hidden = true;
  const result = await api('/api/compare',{ids});
  if (token !== state.runRequest) return;
  state.comparison = result;
  $('compareBaseline').replaceChildren(...result.runs.map((m,i) => {const option = element('option',`${String.fromCharCode(65+i)} · ${runName(m)} · ${runDate(m.id)}`); option.value = i; return option;}));
  $('compareBaseline').value = String(Math.max(0,result.runs.findIndex(m => m.id === baselineId)));
  renderComparison();
}
function renderComparison() {
  if (!state.comparison) return;
  const {runs,rows} = state.comparison, baseline = Number($('compareBaseline').value), metric = $('compareMetric').value;
  const format = $('compareFormat').value;
  const scoped = rows.filter(row => !format || row.cases.some(c => c?.format === format));
  const matched = scoped.filter(row => row.alignment === 'matched'), accepted = matched.filter(allAccepted);
  const formatGroups = Object.entries(FORMAT).filter(([key]) => !format || key === format)
    .map(([key,label]) => ({label,rows:accepted.filter(row => row.cases[baseline].format === key)}));
  const totals = runs.map((_,i) => scoreTotals(accepted.map(row => row.cases[i])));
  const mean = (i,key) => accepted.length ? accepted.reduce((sum,row) => sum + caseValue(row.cases[i],key),0) / accepted.length : null;
  const limits = new Set(runs.map(m => m.config.timeLimitMs ?? 'official'));
  $('compareDescription').textContent = `${runs.length}件を比較 · 同一問題 ${matched.length}seed / ${scoped.length}seed · 全履歴AC ${accepted.length}seed。${limits.size > 1 ? '制限時間の設定が異なります。' : ''}${runs.some(busy) ? '実行途中の記録を含みます。最新の結果は「比較を更新」で取得します。' : ''}`;
  $('compareCards').style.setProperty('--run-count',runs.length);
  $('compareCards').replaceChildren(...runs.map((m,i) => {
    const card = element('section',undefined,`card compare-card${i === baseline ? ' baseline' : ''}`), tag = String.fromCharCode(65+i);
    card.append(element('p',`${tag}${i === baseline ? ' · 比較の基準' : ''}`,'eyebrow'),element('h3',runName(m)),element('p',`${runDate(m.id)} · ${runStatus(m)}`,'note'));
    const hero = element('div',undefined,'outcome-hero');
    if (i === baseline) {
      hero.append(element('span','比較の基準','note'),element('strong',accepted.length.toLocaleString('ja-JP')),element('span','共通AC seed','note'));
    } else {
      const outcomes = winCounts(accepted,i,baseline), grid = element('div',undefined,'outcome-counts'), bar = element('div',undefined,'outcome-bar');
      for (const [key,label,style] of OUTCOMES) {
        const count = outcomes[key], block = element('div',undefined,style);
        block.append(element('strong',accepted.length ? count.toLocaleString('ja-JP') : '—'),element('span',label)); grid.append(block);
        const segment = element('span',undefined,style); segment.style.flexGrow = count; segment.title = `${label} ${count}`; bar.append(segment);
      }
      hero.append(element('p',`基準 ${String.fromCharCode(65+baseline)} に対して · ${FORMAT[format] || '全形式'}`,'note'),grid,bar);
      if (!accepted.length) hero.append(element('p','比較可能な共通ACはありません。','note'));
    }
    card.append(hero);
    if (isGame) {
      const stats=matchStats(accepted.map(r=>r.cases[i])), panel=rankSummary(element,stats,FORMAT[format] || '全体');
      const base=matchStats(accepted.map(r=>r.cases[baseline])).averageRank;
      if(i!==baseline && stats.averageRank!=null && base!=null) panel.append(difference(stats.averageRank-base,true));
      panel.append(opponentBreakdown(element,stats,[m.config.targetName,...m.config.opponents.map(p=>p.name)]));
      card.append(panel);
    }
    const breakdown = element('table',undefined,'format-outcomes'), formatHead = element('thead'), formatBody = element('tbody'), formatHeaders = element('tr');
    breakdown.append(element('caption',i === baseline ? '形式別の共通AC' : '形式別の勝敗'));
    for (const label of ['形式',...(i === baseline ? ['共通AC'] : OUTCOMES.map(([,label]) => label))]) {
      const th = element('th',label); th.scope = 'col'; formatHeaders.append(th);
    }
    formatHead.append(formatHeaders);
    for (const group of formatGroups) {
      const tr = element('tr'), label = element('th',group.label); label.scope = 'row'; tr.append(label);
      if (i === baseline) tr.append(element('td',`${group.rows.length.toLocaleString('ja-JP')}件`));
      else {
        const outcomes = winCounts(group.rows,i,baseline);
        for (const [key,,style] of OUTCOMES) {
          const td = element('td',group.rows.length ? outcomes[key].toLocaleString('ja-JP') : '—',style);
          if (!group.rows.length) td.title = '比較可能な共通ACはありません。';
          tr.append(td);
        }
      }
      formatBody.append(tr);
    }
    breakdown.append(formatHead,formatBody); card.append(breakdown);
    const scorePanel = element('section',undefined,'comparison-totals');
    scorePanel.append(element('h4','スコア合計'),element('p',`${FORMAT[format] || '全形式'} · 共通AC ${accepted.length}件`,'note'));
    const scores = element('dl',undefined,'comparison-total-values');
    for (const [key,label] of [['uniqueBrands','種類数'],['dailyBrands','日別種類'],['udon','玉数']]) {
      const item = element('div'), value = element('dd');
      value.append(element('strong',accepted.length ? totals[i][key].toLocaleString('ja-JP') : '—'));
      if (i !== baseline && accepted.length) {
        const delta = difference(totals[i][key] - totals[baseline][key]);
        delta.title = `基準 ${String.fromCharCode(65+baseline)} との差`;
        value.append(delta);
      } else value.append(element('span',accepted.length ? '基準' : '対象なし','total-reference'));
      item.append(element('dt',label),value); scores.append(item);
    }
    scorePanel.append(scores); card.append(scorePanel);
    const counts = element('div',undefined,'verdict-counts');
    for (const v of ['AC','WA','TLE','RE']) counts.append(element('span',`${v} ${m.cases.filter(c => c.status === v).length}`,v === 'AC' ? 'good' : ''));
    if (m.verdict === 'CE') counts.append(element('span','CE','bad'));
    card.append(counts);
    const details = element('details',undefined,'comparison-details');
    details.append(element('summary','平均・形式別内訳・実行条件'),element('p',`完了 ${m.completed} / ${m.total} · ${FORMAT[m.config.format] || '形式自動決定'}`,'note'),element('p',executionText(m),'note'),element('p',`制限時間 ${m.config.timeLimitMs == null ? '公式条件' : `${m.config.timeLimitMs} ms`}`,'note'));
    const means = element('dl',undefined,'compare-means');
    for (const [key,label] of [['uniqueBrands','平均種類数'],['dailyBrands','平均日別種類'],['udon','平均玉数'],['answerTimeMs','平均回答時間']]) {
      const value = mean(i,key), base = mean(baseline,key), dd = element('dd',value == null ? '—' : `${value.toFixed(2)}${key === 'answerTimeMs' ? ' ms' : ''}`);
      if (i !== baseline && value != null) dd.append(difference(value-base,key === 'answerTimeMs'));
      means.append(element('dt',label),dd);
    }
    details.append(element('p',`共通AC ${accepted.length}seedの平均`,'note'),means,element('p','この実行の合計（全完了ケース）','note'),totalsTable(m.cases,true));
    if(isGame) {
      const ranks=element('dl',undefined,'compare-means');
      for(const s of matchScopeStats(accepted.map(r=>r.cases[i])).filter(s=>!format || !s.format || s.format===format)) ranks.append(element('dt',`${s.label} · ${s.count}試合`),element('dd',averageRankText(s.averageRank)));
      details.append(element('p','共通ACの平均順位（小さいほど上位）','note'),ranks);
    }
    card.append(details);
    if (m.error) card.append(element('p',m.error,'error'));
    const open = element('button','この履歴を開く'); open.onclick = guard(() => openRun(m.id)); card.append(open);
    return card;
  }));
  const filter = $('compareFilter').value;
  const visible = scoped.filter(row => {
    if (filter === 'matched') return row.alignment === 'matched';
    if (filter === 'unmatched') return row.alignment !== 'matched';
    if (filter === 'changed') return row.alignment === 'matched' && row.cases.some(c => c.status !== row.cases[baseline].status || scoreOrder(c,row.cases[baseline]) !== 0);
    return true;
  });
  $('compareSummary').textContent = `表示 ${visible.length}seed。上部の合計・平均・勝敗は、選択した形式の同一問題かつ全履歴ACのケースが対象（ケースの表示条件には連動しません）。勝敗は種類数 → 日別種類 → 玉数の順で比較します。回答時間は並列数や実行環境でも変動します。`;
  const head = element('thead'), headings = element('tr'), body = element('tbody');
  headings.append(element('th','seed / 問題'));
  runs.forEach((m,i) => headings.append(element('th',`${String.fromCharCode(65+i)} · ${runName(m)}`))); head.append(headings);
  const alignLabel = {matched:'同一問題',different:isGame ? '問題・対戦条件相違' : '問題相違',missing:'未実行あり',unavailable:'問題を確認できません'};
  for (const row of visible) {
    const tr = element('tr'), title = element('td');
    title.append(element('strong',row.seed),element('small',alignLabel[row.alignment],row.alignment === 'matched' ? 'note' : 'warn'));
    if (row.alignment === 'matched') title.append(element('small',FORMAT[row.cases[0].format],'note'));
    tr.append(title);
    row.cases.forEach((c,i) => {
      const cell = element('td');
      if (!c) cell.append(element('span',runs[i].verdict === 'CE' ? 'CE · 未実行' : '未実行','note'));
      else {
        const score = element('div',undefined,'compare-value');
        score.append(element('strong',c.status === 'AC' ? caseValue(c,metric) : '—'),element('span',c.status,`verdict ${c.status === 'AC' ? 'good' : 'warn'}`));
        if (i !== baseline && allAccepted(row)) score.append(difference(caseValue(c,metric)-caseValue(row.cases[baseline],metric),metric === 'answerTimeMs'));
        cell.append(score);
        if (isGame) cell.append(caseRankBadge(element,c));
        if (row.alignment !== 'matched') cell.append(element('small',FORMAT[c.format],'note'));
        const replay = element('button',c.error ? '理由' : '再生','quiet');
        replay.setAttribute('aria-label',`${String.fromCharCode(65+i)} seed ${row.seed}を${c.error ? '確認' : '再生'}`);
        replay.onclick = c.error ? () => error(c.error) : guard(() => openRun(runs[i].id,row.seed));
        cell.append(replay);
      }
      tr.append(cell);
    });
    body.append(tr);
  }
  $('comparisonTable').replaceChildren(head,body); $('compareEmpty').hidden = visible.length > 0;
}

function formHandler(errorId,fn) {
  return async event => {
    event.preventDefault(); $(errorId).hidden = true;
    const submit = event.target.querySelector('button[type=submit]');
    submit.disabled = true;
    if (errorId === 'runError') state.submitting = true;
    try {await fn();} catch (e) {$(errorId).textContent = e.message; $(errorId).hidden = false;}
    finally {state.submitting = false; submit.disabled = errorId === 'runError' && !!state.activeId;}
  };
}
async function main() {
  if (isGame) {
    createOpponentFields($('opponentFields'),element);
    for(let id=1;id<=4;id++) $(`opponent-${id}-parallel`).onchange=parallelHint;
    $('playerSelect').onchange=guard(async()=>{
      const t=state.t,player=Number($('playerSelect').value);
      loadCase(state.case,state.matchReplay,player);setTime(t);state.navigation?.update();await loadPlayerLog();
    });
    $('showOthers').onchange=()=>{render();state.navigation?.update();};
  }
  try {state.deletedIds = new Set(JSON.parse(sessionStorage.getItem(storagePrefix+'.deleted-runs') || '[]'));} catch { /* No prior deletions. */ }
  const legend = [...BOARD_LEGEND,['巡回車（円・燃料バー）',isGame ? '#64748b' : AGENT_COLOR[0]],['補給車（四角）',AGENT_COLOR[1]]];
  $('legend').replaceChildren(...legend.map(([label,color]) => {const item = element('div',undefined,'legend-item'), swatch = element('span',undefined,'swatch'); swatch.style.background = color; item.append(swatch,element('span',label)); return item;}));
  document.querySelectorAll('[data-view]').forEach(tab => tab.onclick = guard(() => switchView(tab.dataset.view)));
  document.querySelectorAll('[data-close]').forEach(button => button.onclick = () => $(button.dataset.close).close());
  $('mode').onchange = () => {const single = $('mode').value === 'single'; $('countRow').hidden = single; $('seedRow').hidden = !single; $('count').disabled = single; $('seed').disabled = !single; parallelHint();};
  for (const id of ['count','jobs','solverParallel']) $(id).oninput = parallelHint;
  $('buildEnabled').onchange = () => {$('run').textContent = $('buildEnabled').checked ? 'ビルドして実行' : '実行';};
  $('newRun').onclick = $('emptyRun').onclick = () => openRunDialog();
  $('chooseComparison').onclick = chooseComparison;
  $('cancelComparison').onclick = () => {state.compareMode = false; renderHistory();};
  $('compareSelected').onclick = guard(openComparison);
  $('refreshComparison').onclick = guard(() => state.navigation.go(captureRoute(state.navigation.route),{replace:true}));
  for (const id of ['compareBaseline','compareMetric','compareFilter','compareFormat']) $(id).oninput = () => {renderComparison(); state.navigation.update();};
  $('rerun').onclick = () => openRunDialog(true);
  $('newPreview').onclick = () => {$('previewError').hidden = true; $('previewDialog').showModal();};
  $('runForm').onsubmit = formHandler('runError',startRun); $('previewForm').onsubmit = formHandler('previewError',generateCase);
  $('deleteForm').onsubmit = formHandler('deleteError',deleteHistory);
  $('importHistory').onclick = () => {$('importForm').reset(); $('importError').hidden = true; $('importDialog').showModal();};
  $('importForm').onsubmit = formHandler('importError',importHistory);
  $('importDialog').addEventListener('cancel',event => {if (state.importing) event.preventDefault();});
  $('exportRun').onclick = guard(() => exportHistory(state.selectedRunId));
  $('cancel').onclick = guard(async () => {if (state.activeId) {await api(`/api/runs/${state.activeId}/cancel`,{}); $('cancel').disabled = true;}});
  $('showActive').onclick = guard(() => state.activeId && openRun(state.activeId));
  $('refresh').onclick = guard(async () => {
    await refreshHistory();
    const id = state.selectedRunId, token = state.runRequest;
    if (id) {const result = await api(`/api/runs/${id}`); if (token === state.runRequest) renderRun(result);}
  });
  for (const id of ['filterFormat','filterStatus','filterSeed','sortCases']) $(id).oninput = () => {renderResults(); updateCaseNavigation(); state.navigation.update();};
  $('backToResults').onclick = guard(() => switchView('results'));
  $('previousCase').onclick = guard(() => adjacentCase(-1)); $('nextCase').onclick = guard(() => adjacentCase(1));
  $('saveProblem').onclick = () => download(`problem-${state.case.seed ?? 'import'}.json`,state.case.problem);
  $('download').onclick = () => download(`replay-${state.case.seed ?? 'import'}.json`,state.matchReplay || state.replay);
  $('exportGif').onclick = openGifDialog; $('gifForm').onsubmit = makeGif;
  for (const id of ['gifScope','gifSpeed','gifWidth']) $(id).oninput = gifHint;
  $('gifDialog').addEventListener('close',() => {state.gifAbort?.abort();});
  $('toggleInspector').onclick = () => setInspectorOpen(!state.inspectorOpen);
  $('closeInspector').onclick = () => {setInspectorOpen(false); $('toggleInspector').focus();};
  const inspectorTabs = [...document.querySelectorAll('[data-inspector]')];
  inspectorTabs.forEach((button,i) => {
    button.onclick = () => setInspectorTab(button.dataset.inspector);
    button.onkeydown = e => {if (['ArrowLeft','ArrowRight'].includes(e.key)) {e.preventDefault(); const next = inspectorTabs[(i + (e.key === 'ArrowLeft' ? -1 : 1) + inspectorTabs.length) % inspectorTabs.length]; setInspectorTab(next.dataset.inspector); next.focus();}};
  });
  setInspectorTab(state.inspectorTab); setInspectorOpen(state.inspectorOpen);
  let resizeFrame;
  new ResizeObserver(() => {cancelAnimationFrame(resizeFrame); resizeFrame = requestAnimationFrame(resizeBoard);}).observe($('canvas').parentElement);
  $('replayFile').onchange = guard(async () => {
    const file = $('replayFile').files[0]; if (!file) return;
    if (file.size > 64 * 1024 * 1024) throw new Error('ファイルは64 MiB以下にしてください。');
    const data = JSON.parse(await file.text());
    if (data.schemaVersion !== undefined && (data.schemaVersion !== (isGame ? 2 : 1) || !data.case)) throw new Error('対応していないリプレイ形式です。');
    const id = crypto.randomUUID(); state.imports.set(id,{name:file.name,data});
    await state.navigation.go({kind:'import',id});
    $('replayFile').value = '';
  });
  $('play').onclick = () => {if (!state.playing && state.t === state.timeline.length - 1) setTime(0); lastTick = performance.now(); setPlaying(!state.playing);};
  $('prev').onclick = () => {setPlaying(false); setTime(state.t - 1);}; $('next').onclick = () => {setPlaying(false); setTime(state.t + 1);};
  $('slider').oninput = () => {setPlaying(false); setTime(Number($('slider').value));};
  $('slider').onpointermove = seekHover;
  $('slider').onpointerleave = $('slider').onblur = () => {$('seekTooltip').hidden = true;};
  $('day').onchange = () => {setPlaying(false); setTime(state.timeline.findIndex(t => t.d === Number($('day').value)));};
  $('step').onchange = () => {setPlaying(false); const start = state.timeline.findIndex(t => t.d === Number($('day').value)); const length = state.days[Number($('day').value)].frames.length; setTime(start + Math.min(length - 1,Math.max(0,Number($('step').value) || 0)));};
  $('step').onkeydown = e => {if (e.key === 'Enter') $('step').onchange();};
  $('toggleHeatmap').onclick = () => {state.heatmapShown = !state.heatmapShown; render(); state.navigation.update();};
  for (const id of ['heatmapMode','heatmapScope','heatmapTeams']) $(id).onchange = () => {render(); state.navigation.update();};
  $('showIds').onchange = $('showRoute').onchange = () => {render(); state.navigation.update();};
  $('speed').onchange = () => state.navigation.update();
  bindBoardControls({canvas:$('canvas'),camera,zoomIn:$('zoomIn'),zoomOut:$('zoomOut'),zoomReset:$('zoomReset'),
    redraw:() => {if (state.case) renderMap();},onSelect:e => {
      if (!state.case) return;
      const canvas = $('canvas'), [x,y] = boardPoint(canvas,e,camera);
      const L = layout(state.case.problem,canvas.width - 8,canvas.height - 8); let best = null, distance = Infinity;
      for (let r = 0; r < L.H; r++) for (let c = 0; c < L.W; c++) {const [cx,cy] = L.center(r,c), dist = (cx-x)**2 + (cy-y)**2; if (dist < distance) {best = r * L.W + c; distance = dist;}}
      state.selectedCell = distance < L.s * L.s ? best : null; render(); state.navigation.update();
    }});
  requestAnimationFrame(animate); setInterval(poll,1000);
  const [config,system] = await Promise.all([api('/api/config'),api('/api/system')]);
  state.availableCpus = system.availableCpus; state.localConfig = structuredClone(config); fillConfig(config);
  $('previewArenaMaps').checked = config.arenaMaps !== false;
  await refreshHistory();
  state.navigation = new Navigation({window,capture:captureRoute,render:renderRoute,onError:e => error(e.message),normalize:route => withoutDeletedRuns(route,state.deletedIds,state.runs[0]?.id),persist:route => {
    try {sessionStorage.setItem(storagePrefix+'.navigation.v2',JSON.stringify(route));} catch { /* URL remains usable without storage. */ }
  }});
  let saved = null;
  try {saved = JSON.parse(sessionStorage.getItem(storagePrefix+'.navigation.v2'));} catch { /* Start from the URL or latest record. */ }
  const fromUrl = parseRoute(location.hash);
  const inHistory = history.state?.testerNavigation === 1 ? history.state.route : null;
  const initial = fromUrl ? {...(inHistory || {}),...fromUrl} : saved || (state.runs[0] ? {kind:'run',id:state.runs[0].id,view:'results'} : {kind:'home'});
  await state.navigation.go(initial,{replace:true});
  let scrollTimer;
  window.addEventListener('scroll',() => {clearTimeout(scrollTimer); scrollTimer = setTimeout(() => state.navigation.update(),150);},{passive:true});
  window.addEventListener('pagehide',() => state.navigation.update());
}
guard(main)();

function playerNames() {
  return [state.runView?.manifest.config.targetName || '対象ソルバ',...(state.runView?.manifest.config.opponents || [1,2,3,4].map(id=>({name:`プレイヤー${id}`}))).map(p=>p.name)];
}
function renderStandings(day,step) {
  renderPlayerCards({container:$('matchStandings'),replay:state.matchReplay,names:playerNames(),player:state.player,day,step,element,
    onSelect:id=>{$('playerSelect').value=id;$('playerSelect').onchange();}});
}
async function loadPlayerLog() {
  const id=state.selectedRunId,seed=state.openedSeed,player=state.player;
  if(!id || seed==null)return;
  $('solverLogTitle').textContent=`seed ${seed} · プレイヤー${player} のソルバログ（stderr）`;
  const response=await fetch(`/api/runs/${id}/${seed}/stderr?player=${player}`);
  const text=response.ok?(await response.text())||'stderrへの出力はありません。':'ログを読み込めませんでした。';
  if(id===state.selectedRunId && seed===state.openedSeed && player===state.player)$('solverLog').textContent=text;
}
