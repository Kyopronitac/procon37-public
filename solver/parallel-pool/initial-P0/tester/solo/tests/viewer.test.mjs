import test from 'node:test';
import assert from 'node:assert/strict';
import {boardPoint,bindBoardControls} from '../../../common/web/viewer.js';

test('cell coordinates are correct after zoom, pan and device pixel scaling',() => {
  const canvas = {width:1200,height:800,getBoundingClientRect:() => ({left:10,top:20,width:600,height:400})};
  // Logical point (504,304) becomes canvas point (378,174) under this camera.
  const point = boardPoint(canvas,{clientX:199,clientY:107},{zoom:2.5,x:18,y:14});
  assert.deepEqual(point,[500,300]);
});

test('zoom is bounded, dragging does not select a cell, and reset restores the full board',() => {
  const camera = {zoom:1,x:0,y:0}, controls = {zoomIn:{},zoomOut:{},zoomReset:{}};
  const canvas = {width:1000,height:600,getBoundingClientRect:() => ({width:500,height:300}),setPointerCapture() {},releasePointerCapture() {}};
  let clicks = 0;
  bindBoardControls({canvas,camera,...controls,redraw() {},onSelect() {clicks++;}});
  for (let i = 0; i < 20; i++) controls.zoomIn.onclick(); assert.equal(camera.zoom,6);
  canvas.onpointerdown({button:0,pointerId:1,clientX:100,clientY:100});
  canvas.onpointermove({pointerId:1,clientX:130,clientY:140});
  canvas.onpointerup({pointerId:1}); assert.equal(clicks,0); assert.deepEqual([camera.x,camera.y],[60,80]);
  canvas.onpointerdown({button:0,pointerId:1,clientX:100,clientY:100}); canvas.onpointerup({pointerId:1}); assert.equal(clicks,1);
  controls.zoomReset.onclick(); assert.deepEqual(camera,{zoom:1,x:0,y:0});
  controls.zoomOut.onclick(); assert.equal(camera.zoom,1);
});
