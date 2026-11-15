import test from 'node:test';
import assert from 'node:assert/strict';
import {matchStats, matchScopeStats, averageRankText, caseRankBadge, playerReplays, drawOtherPlayers, PLAYER_COLORS} from '../web/match-ui.js';
import {routeHash, parseRoute} from '../../../common/web/navigation.js';
import {rerunConfig} from '../../../common/web/run-config.js';
const score = (a,b,c) => ({uniqueBrands:a,dailyBrands:b,udon:c});
test('match statistics use all-AC cases and lexicographic scores, never answer time', () => {
  const players=[score(3,2,1),score(3,2,1),score(2,9,9),score(3,3,0),score(3,2,0)].map((score,id)=>({score,rank:[2,2,5,1,4][id],status:'AC',answerTimeMs:500-id}));
  const actual=matchStats([{status:'AC',players},{status:'TLE',players},{status:'AC'}]);
  assert.deepEqual(actual,{count:1,ranks:[0,1,0,0,0],opponents:[[0,1,0],[1,0,0],[0,0,1],[1,0,0]],averageRank:2,playerAverageRanks:[2,2,5,1,4]});
});
test('average ranks use individual all-AC matches, preserve ties and separate formats', () => {
  const make=(format,ranks)=>({format,status:'AC',players:ranks.map(rank=>({rank,status:'AC',score:score(6-rank,0,0)}))});
  const cases=[make('first',[1,1,3,4,5]),make('first',[3,1,2,4,5]),make('semifinal',[5,4,3,2,1])];
  const failed=make('final',[1,1,1,1,1]);failed.players[3].status='TLE';
  const malformed=make('final',[0,1,1,1,1]);
  const groups=matchScopeStats([...cases,{...cases[0],status:'RE'},failed,malformed,{status:'AC'}]);
  assert.deepEqual(groups.map(s=>s.count),[3,2,1,0]);
  assert.deepEqual(groups.map(s=>s.averageRank),[3,2,5,null]);
  assert.deepEqual(groups[0].playerAverageRanks,[3,2,8/3,10/3,11/3]);
  assert.deepEqual(groups[0].ranks,[1,0,1,0,1]);
  assert.deepEqual(matchStats([]).playerAverageRanks,[null,null,null,null,null]);
  assert.equal(averageRankText(null),'—');assert.equal(averageRankText(8/3),'2.67位');
});
test('case badges make ties and fallback ranks explicit; rank sorting survives navigation', () => {
  const element=(tag,text,className)=>({tag,text,className,children:[],append(child){this.children.push(child);}});
  const c={status:'AC',players:[{rank:1},{rank:1},{rank:3},{rank:4},{rank:5}]};
  const badge=caseRankBadge(element,c);
  assert.equal(badge.text,'同率1位');assert.equal(badge.className,'rank-badge rank-1');
  assert.equal(caseRankBadge(element,{...c,status:'TLE'}).children[0].text,'参考');
  assert.equal(caseRankBadge(element,{status:'RE'}).text,'—');
  assert.equal(parseRoute(routeHash({kind:'run',id:'123-45',view:'results',filters:{sort:'rank'}})).filters.sort,'rank');
});
test('selected player and overlay survive URL navigation', () => {
  const r={kind:'run',id:'123-45',view:'map',seed:'0',player:4,showOthers:false,frame:10};
  const parsed=parseRoute(routeHash(r));
  assert.equal(parsed.player,4);assert.equal(parsed.showOthers,false);assert.equal(parsed.frame,10);
});
test('overlay draws all other teams and honors visibility', () => {
  const replay={days:[{frames:[{agents:[{pos:1,kind:0}]}]}]};
  const match={...replay,opponents:[replay,replay,replay,replay]};
  assert.equal(playerReplays(match).length,5);
  const colors=[];
  const ctx={beginPath(){},arc(){},rect(){},fillText(){},fill(){if (this.fillStyle !== 'rgba(255,255,255,0.15)') colors.push(this.fillStyle);},stroke(){}};
  const settings={showOthers:true,matchReplay:match,player:3};
  drawOtherPlayers(ctx,()=>[0,0],10,settings,1,0);
  assert.deepEqual(colors,PLAYER_COLORS.filter((_,id) => id !== settings.player));
  drawOtherPlayers(ctx,()=>[0,0],10,{...settings,showOthers:false},1,0);
  assert.equal(colors.length,4);
});
test('imported reruns use local commands for every opponent', () => {
  const local={solver:{program:'local'},build:null,opponents:Array.from({length:4},(_,i)=>({solver:{program:`local-${i}`},build:null}))};
  const manifest={importedFrom:'other',config:{mode:'game',targetName:'candidate',solver:{program:'foreign'},opponents:local.opponents.map(()=>({solver:{program:'foreign'}}))}};
  const config=rerunConfig(manifest,local);
  assert.deepEqual(config.opponents,local.opponents);assert.equal(config.solver.program,'local');assert.equal(config.targetName,'candidate');
});
