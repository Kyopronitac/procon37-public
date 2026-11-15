export function planGifFrames(days,{day = null,speed = 8} = {}) {
  if (![1,2,4,8,16].includes(speed) || (day !== null && (!Number.isInteger(day) || !days[day]))) throw new Error('GIFの出力設定が不正です。');
  const stride = Math.max(1,Math.ceil(4 * speed / 12)), frames = [];
  let offset = 0;
  days.forEach((d,index) => {
    if (day === null || day === index) {
      for (let s = 0; s < d.frames.length; s += stride) frames.push({index:offset+s,day:index,step:s});
      const end = d.frames.length - 1;
      if (frames.at(-1)?.index !== offset + end) frames.push({index:offset+end,day:index,step:end});
    }
    offset += d.frames.length;
  });
  return frames.map((frame,i) => ({...frame,delay:i === frames.length - 1 ? 600 : Math.max(20,Math.round((frames[i+1].index - frame.index) * 25 / speed) * 10)}));
}

// Only one RGBA frame is in flight; encoding never blocks the UI thread.
export async function exportGif({width,height,frames,draw,signal,onProgress}) {
  signal.throwIfAborted();
  const worker = new Worker(new URL('./gif-worker.js',import.meta.url),{type:'module'});
  let pendingReject;
  const abort = () => {worker.terminate(); pendingReject?.(new DOMException('GIF出力を中止しました。','AbortError'));};
  signal.addEventListener('abort',abort,{once:true});
  const send = (message,transfer = []) => new Promise((resolve,reject) => {
    signal.throwIfAborted(); pendingReject = reject;
    worker.onmessage = event => {pendingReject = null; event.data.error ? reject(new Error(event.data.error)) : resolve(event.data);};
    worker.onerror = event => {event.preventDefault(); reject(new Error(event.message || 'GIFの生成に失敗しました。'));};
    worker.postMessage(message,transfer);
  });
  try {
    for (let i = 0; i < frames.length; i++) {
      signal.throwIfAborted();
      const pixels = draw(frames[i]);
      await send({type:'frame',width,height,delay:frames[i].delay,buffer:pixels.data.buffer},[pixels.data.buffer]);
      onProgress(i+1,frames.length);
    }
    const {buffer} = await send({type:'finish'});
    return new Blob([buffer],{type:'image/gif'});
  } finally {
    signal.removeEventListener('abort',abort); worker.terminate();
  }
}
