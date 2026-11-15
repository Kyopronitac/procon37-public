import test from 'node:test';
import assert from 'node:assert/strict';
import {matchStats, matchScopeStats, averageRankText, caseRankBadge, teamCell, playerReplays, drawOtherPlayers, drawOtherPlayerRoutes, renderPlayerCards, PLAYER_COLORS, MAX_PLAYERS, playerColor} from '../web/match-ui.js';
import {drawDayRoutes} from '../../../common/web/board.js';
import {routeHash, parseRoute} from '../../../common/web/navigation.js';
const score = (a,b,c) => ({uniqueBrands:a,dailyBrands:b,udon:c});
const element = (tag,text,className) => ({tag,text,className,children:[],title:'',append(...nodes){this.children.push(...nodes);}});

test('rank statistics follow the team count and use all-AC matches only', () => {
  const players = ranks => ranks.map((rank,id) => ({rank,status:'AC',score:score(4-rank,id,0)}));
  const cases = [{format:'first',status:'AC',players:players([1,2,3])},{format:'final',status:'AC',players:players([3,1,2])}];
  const failed = {format:'first',status:'TLE',players:players([1,2,3])}; failed.players[2].status = 'TLE';
  const stats = matchStats([...cases,failed,{status:'AC'}]);
  assert.equal(stats.count,2);
  assert.deepEqual(stats.averageRanks,[2,1.5,2.5]);
  assert.deepEqual(stats.ranks,[[1,0,1],[1,1,0],[0,1,1]]);
  assert.deepEqual(stats.versus,[[1,0,1],[1,0,1]]);
  assert.deepEqual(matchScopeStats(cases).map(s => s.count),[2,1,0,1]);
  assert.deepEqual(matchStats([]).averageRanks,[]);
  assert.deepEqual(matchStats([],2).averageRanks,[null,null]);
  assert.equal(averageRankText(1.5),'1.50位');
});

test('team cells show each team rank, score and its own verdict', () => {
  const c = {status:'TLE',players:[{rank:1,status:'AC',answerTimeMs:5,score:score(3,4,5)},{rank:1,status:'AC',answerTimeMs:5,score:score(3,4,5)},{rank:3,status:'TLE',answerTimeMs:0,score:score(0,0,0)}]};
  const tied = caseRankBadge(element,c,0);
  assert.equal(tied.text,'同率1位'); assert.equal(tied.className,'rank-badge rank-1'); assert.equal(tied.children[0].text,'参考');
  const cell = teamCell(element,c,2);
  assert.deepEqual(cell.children.map(n => n.text),['3位','0 · 0 · 0','TLE']);
  // Ranks beyond five use the neutral badge instead of an undefined color class.
  const many = {status:'AC',players:Array.from({length:8},(_,i) => ({rank:i+1,status:'AC',score:score(8-i,0,0)}))};
  assert.equal(caseRankBadge(element,many,7).className,'rank-badge rank-none');
  assert.equal(teamCell(element,{status:'AC'},0).children[0].text,'—');
});

test('replays of any team count are drawn with distinct team colors', () => {
  assert.equal(MAX_PLAYERS,16); assert.equal(new Set(PLAYER_COLORS).size,16); assert.equal(playerColor(16),PLAYER_COLORS[0]);
  const replay = {days:[{frames:[{agents:[{pos:1,kind:0}]}]}]};
  assert.deepEqual(playerReplays(null),[]);
  assert.equal(playerReplays(replay).length,1);
  const match = {...replay,opponents:Array(15).fill(replay)};
  const colors = [];
  const ctx = {beginPath(){},arc(){},rect(){},fillText(){},fill(){if (this.fillStyle !== 'rgba(255,255,255,0.15)') colors.push(this.fillStyle);},stroke(){}};
  drawOtherPlayers(ctx,() => [0,0],10,{showOthers:true,matchReplay:match,player:15},1,5);
  assert.deepEqual(colors,PLAYER_COLORS.slice(0,15));
  assert.equal(new Set(colors).size,15);
});

test('daily routes for sixteen teams use the same palette as player cards',() => {
  const replay = {score:score(1,1,1),days:[{day:1,frames:[
    {agents:[{pos:0,kind:0}],score:score(1,1,1)},
    {agents:[{pos:1,kind:0}],score:score(1,1,1)},
  ]}]};
  const match = {...replay,opponents:Array(15).fill(replay)}, cards = [];
  const makeElement = (...args) => ({...element(...args),style:{setProperty(key,value){this[key] = value;}},setAttribute(){}});
  renderPlayerCards({container:{replaceChildren(...nodes){cards.push(...nodes);}},replay:match,names:[],player:15,day:0,step:0,element:makeElement,onSelect(){}});
  assert.deepEqual(cards.map(card => card.style['--player-color']),PLAYER_COLORS);
  const strokes = [], ctx = {save(){},restore(){},setLineDash(){},beginPath(){},moveTo(){},lineTo(){},stroke(){strokes.push(this.strokeStyle);}};
  const center = pos => [pos,0];
  drawOtherPlayerRoutes(ctx,center,20,{showRoute:true,showOthers:true,matchReplay:match,player:15},1);
  drawDayRoutes(ctx,center,20,replay.days[0],() => playerColor(15));
  assert.equal(new Set(strokes).size,16);
  const actual = strokes.splice(0);
  cards.forEach(card => drawDayRoutes(ctx,center,20,replay.days[0],() => card.style['--player-color']));
  assert.deepEqual(actual,strokes);
});

test('team selection beyond the fifth player survives URL navigation', () => {
  const route = {kind:'run',id:'123-45',view:'map',seed:'0',player:12,showOthers:false,frame:10};
  const parsed = parseRoute(routeHash(route));
  assert.equal(parsed.player,12); assert.equal(parsed.showOthers,false);
  assert.equal(parseRoute('#run=123-45&player=16').player,undefined);
  assert.equal(parseRoute('#run=123-45&player=-1').player,undefined);
});
