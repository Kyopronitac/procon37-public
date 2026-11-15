import test from 'node:test';
import assert from 'node:assert/strict';
import {Navigation,parseRoute,routeHash,withoutDeletedRuns} from '../web/navigation.js';
import {scoreTotals,winCounts} from '../web/record-stats.js';
import {playbackChapters,chapterProgress} from '../web/playback.js';
import {planGifFrames} from '../web/gif-export.js';
import {rerunConfig} from '../web/run-config.js';

test('imported reruns use local commands including for previously saved archives',() => {
  const untrusted = {program:'/bin/sh',args:['-c','echo archive-command'],cwd:'/',env:{BASH_ENV:'/archive/startup'}};
  const saved = {count:60,seed:null,format:'final',arenaMaps:false,jobs:4,label:'imported',timeLimitMs:null,solverParallel:true,build:untrusted,solver:untrusted};
  for (const build of [null,{program:'cargo',args:['build'],cwd:'/local',env:{}}]) {
    const local = {build,solver:{program:'/local/solver',args:['0'],cwd:'/local',env:{}},solverParallel:false};
    const config = rerunConfig({importedFrom:'123-1',config:saved},local);
    assert.deepEqual(config,{...saved,build:local.build,solver:local.solver,solverParallel:false});
    config.solver.args.push('edited');
    assert.deepEqual(local.solver.args,['0']);
    assert.deepEqual(saved.solver,untrusted);
  }
  assert.deepEqual(rerunConfig({config:saved},null),saved);
});

function browser() {
  const entries = [{state:null,url:'/'}], listeners = new Map(); let index = 0;
  const w = {location:{pathname:'/',search:'',hash:''},addEventListener:(name,fn) => listeners.set(name,fn),scrollTo:() => {}};
  const select = () => {const url = new URL(entries[index].url,'http://localhost'); w.location.hash = url.hash;};
  w.history = {
    get state() {return entries[index].state;},
    pushState(state,_,url) {entries.splice(++index); entries.push({state:structuredClone(state),url}); select();},
    replaceState(state,_,url) {entries[index] = {state:structuredClone(state),url}; select();}
  };
  w.move = async offset => {index += offset; select(); await listeners.get('popstate')({state:w.history.state});};
  w.entries = entries;
  return w;
}

test('browser back and forward restore filters, comparison baseline, and replay position',async () => {
  const window = browser(); let ui = {};
  const navigation = new Navigation({window,capture:() => structuredClone(ui),render:async route => {ui = structuredClone(route);},onError:e => {throw e;}});
  await navigation.go({kind:'run',id:'101-1',view:'results'},{replace:true});
  ui.filters = {status:'RE',format:'final',search:'7',sort:'score'}; navigation.update();
  await navigation.go({kind:'compare',ids:['101-1','102-1'],baseline:'102-1',metric:'udon',format:'final',filter:'changed'});
  await navigation.go({kind:'run',id:'102-1',view:'map',seed:'17'});
  ui.frame = 70; navigation.update();
  assert.equal(window.entries.length,3);
  await window.move(-1);
  assert.equal(ui.kind,'compare'); assert.equal(ui.baseline,'102-1'); assert.equal(ui.metric,'udon'); assert.equal(ui.filter,'changed');
  await window.move(-1);
  assert.deepEqual(ui.filters,{status:'RE',format:'final',search:'7',sort:'score'});
  await window.move(1); await window.move(1);
  assert.equal(ui.seed,'17'); assert.equal(ui.frame,70); assert.equal(window.entries.length,3);
});

test('a late response cannot replace the view reached using back',async () => {
  const window = browser(); let ui,finish;
  const navigation = new Navigation({window,capture:() => ui,render:async (route,current) => {
    if (route.id === '102-1') await new Promise(resolve => {finish = resolve;});
    if (current()) ui = route;
  },onError:e => {throw e;}});
  await navigation.go({kind:'run',id:'101-1',view:'results'},{replace:true});
  const loading = navigation.go({kind:'run',id:'102-1',view:'results'});
  await window.move(-1); finish(); await loading;
  assert.equal(ui.id,'101-1'); assert.equal(window.history.state.route.id,'101-1');
});

test('URLs preserve a u64 seed and reject unsafe run paths',() => {
  const route = {kind:'run',id:'101-1',view:'map',seed:'18446744073709551615',frame:44,filters:{status:'RE',format:'final',search:'15',sort:'issues'}};
  assert.deepEqual(parseRoute(routeHash(route)),route);
  assert.equal(parseRoute('#run=../secret'),null);
  assert.equal(parseRoute('#run=101-1&seed=18446744073709551616').seed,null);
});

test('preview URLs retain arena map exclusion when reloaded or shared',() => {
  const route = {kind:'preview',seed:'11',format:'final',arenaMaps:false,frame:0};
  assert.deepEqual(parseRoute(routeHash(route)),route);
  assert.notEqual(parseRoute(routeHash({kind:'preview',seed:'11',format:'final'})).arenaMaps,false);
});

const c = (uniqueBrands,dailyBrands,udon,status='AC') => ({format:'first',status,score:{uniqueBrands,dailyBrands,udon},answerTimeMs:0});
test('totals include every completed case and sum each score independently',() => {
  assert.deepEqual(scoreTotals([c(2,5,9),c(3,8,10,'RE')]),{uniqueBrands:5,dailyBrands:13,udon:19});
  assert.deepEqual(scoreTotals([]),{uniqueBrands:0,dailyBrands:0,udon:0});
});
test('wins follow score priority and exclude non-AC or different problems',() => {
  const base = c(10,40,100);
  const rows = [c(11,1,1),c(10,41,1),c(10,40,101),c(10,40,100),c(9,100,999),c(100,100,100,'RE')].map(candidate => ({alignment:'matched',cases:[base,candidate]}));
  rows.push({alignment:'different',cases:[base,c(100,100,100)]});
  assert.deepEqual(winCounts(rows,1,0),{wins:3,draws:1,losses:1});
  assert.deepEqual(winCounts(rows,0,1),{wins:1,draws:1,losses:3});
});

test('deletion updates comparisons and browser back cannot reopen a deleted run',async () => {
  const window = browser(), deleted = new Set(); let ui;
  const navigation = new Navigation({window,capture:() => ui,render:async route => {ui = route;},onError:e => {throw e;},
    normalize:route => withoutDeletedRuns(route,deleted,'102-1')});
  await navigation.go({kind:'run',id:'101-1',view:'map',seed:'0'},{replace:true});
  await navigation.go({kind:'compare',ids:['101-1','102-1','103-1'],baseline:'101-1',metric:'udon'});
  deleted.add('101-1');
  await navigation.go(ui,{replace:true});
  assert.deepEqual(ui.ids,['102-1','103-1']); assert.equal(ui.baseline,'102-1'); assert.equal(ui.metric,'udon');
  await window.move(-1);
  assert.deepEqual(ui,{kind:'run',id:'102-1',view:'results'});
  assert.equal(parseRoute(window.location.hash).id,'102-1');
  assert.deepEqual(withoutDeletedRuns({kind:'compare',ids:['101-1','102-1']},deleted,'103-1'),{kind:'run',id:'102-1',view:'results'});
  assert.deepEqual(withoutDeletedRuns({kind:'run',id:'101-1'},deleted,null),{kind:'home'});
});

test('day chapters align with every frame, including unequal days and day boundaries',() => {
  const days = [3,5,2].map((length,i) => ({day:i+1,frames:Array(length)}));
  const chapters = playbackChapters(days), last = 9;
  assert.deepEqual(chapters.map(c => [c.start,c.end]),[[0,2],[3,7],[8,9]]);
  assert.equal(chapters[0].from,0); assert.equal(chapters.at(-1).to,1);
  for (let frame = 0; frame <= last; frame++) {
    const chapter = chapters.find(c => frame >= c.start && frame <= c.end);
    assert.ok(frame / last >= chapter.from && frame / last <= chapter.to);
  }
  assert.equal(chapters[0].to,chapters[1].from);
  assert.equal(chapterProgress(chapters[0],3,last),1);
  assert.equal(chapterProgress(chapters[1],2,last),0);
  assert.ok(chapterProgress(chapters[1],5,last) > 0 && chapterProgress(chapters[1],5,last) < 1);
  assert.equal(chapterProgress(chapters.at(-1),9,last),1);
  const preview = playbackChapters([{day:1,frames:[{}]}]);
  assert.deepEqual(preview,[{day:1,start:0,end:0,from:0,to:1}]);
  assert.equal(chapterProgress(preview[0],0,0),0);
});

test('GIF sampling includes every day boundary and final score at all speeds',() => {
  const days = [8,4,1].map(length => ({frames:Array(length)}));
  for (const speed of [1,2,4,8,16]) {
    const frames = planGifFrames(days,{speed});
    for (const boundary of [0,7,8,11,12]) assert.ok(frames.some(frame => frame.index === boundary));
    assert.equal(frames.at(-1).delay,600);
    assert.ok(frames.every(frame => frame.delay >= 20 && frame.delay % 10 === 0));
    assert.equal(new Set(frames.map(frame => frame.index)).size,frames.length);
  }
  const currentDay = planGifFrames(days,{day:1,speed:1});
  assert.deepEqual(currentDay.map(frame => frame.index),[8,9,10,11]);
  assert.equal(currentDay[0].step,0); assert.ok(currentDay.every(frame => frame.day === 1));
  assert.throws(() => planGifFrames(days,{day:9}));
});
