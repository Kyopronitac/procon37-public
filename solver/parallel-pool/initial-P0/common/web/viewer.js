// 盤面のズーム・ドラッグ操作。単独・対戦・対戦サーバーで共有する。
export function boardPoint(canvas,event,camera) {
  const rect = canvas.getBoundingClientRect(), x = (event.clientX - rect.left) * canvas.width / rect.width, y = (event.clientY - rect.top) * canvas.height / rect.height;
  return [(x - canvas.width / 2 - camera.x) / camera.zoom + canvas.width / 2 - 4,
    (y - canvas.height / 2 - camera.y) / camera.zoom + canvas.height / 2 - 4];
}

export function bindBoardControls({canvas,camera,zoomIn,zoomOut,zoomReset,redraw,onSelect}) {
  const clampPan = () => {
    camera.x = Math.max(-canvas.width * (camera.zoom - 1) / 2,Math.min(canvas.width * (camera.zoom - 1) / 2,camera.x));
    camera.y = Math.max(-canvas.height * (camera.zoom - 1) / 2,Math.min(canvas.height * (camera.zoom - 1) / 2,camera.y));
  };
  const zoom = factor => {camera.zoom = Math.max(1,Math.min(6,camera.zoom * factor)); clampPan(); redraw();};
  zoomIn.onclick = () => zoom(1.2); zoomOut.onclick = () => zoom(1 / 1.2);
  zoomReset.onclick = () => {Object.assign(camera,{zoom:1,x:0,y:0}); redraw();};
  let drag = null;
  canvas.onpointerdown = event => {
    if (event.button !== 0) return;
    drag = {id:event.pointerId,x:event.clientX,y:event.clientY,fromX:camera.x,fromY:camera.y,moved:false};
    canvas.setPointerCapture(event.pointerId);
  };
  canvas.onpointermove = event => {
    if (!drag || event.pointerId !== drag.id) return;
    const dx = event.clientX - drag.x, dy = event.clientY - drag.y;
    drag.moved ||= Math.hypot(dx,dy) > 4;
    if (!drag.moved) return;
    const rect = canvas.getBoundingClientRect();
    camera.x = drag.fromX + dx * canvas.width / rect.width; camera.y = drag.fromY + dy * canvas.height / rect.height;
    clampPan(); redraw();
  };
  canvas.onpointerup = event => {
    if (!drag || event.pointerId !== drag.id) return;
    const select = !drag.moved; drag = null; canvas.releasePointerCapture(event.pointerId);
    if (select) onSelect(event);
  };
  canvas.onpointercancel = () => {drag = null;};
}
