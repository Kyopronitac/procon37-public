// Native range values are frame indices, including each day's initial frame.
export function playbackChapters(days) {
  const last = days.reduce((sum,day) => sum + day.frames.length,0) - 1;
  let start = 0;
  return days.map((day,i) => {
    const end = start + day.frames.length - 1;
    const chapter = {day:day.day,start,end,
      from:i === 0 || last <= 0 ? 0 : (start - 0.5) / last,
      to:i === days.length - 1 || last <= 0 ? 1 : (end + 0.5) / last};
    start = end + 1;
    return chapter;
  });
}

export function chapterProgress(chapter,frame,last) {
  if (last <= 0) return 0;
  return Math.max(0,Math.min(1,(frame / last - chapter.from) / (chapter.to - chapter.from)));
}

// フレーム間の端数を保持し、描画頻度や再生速度によらず同じ速度で進める。
export function advancePlayback(frame,progress,elapsedMs,speed,last) {
  const position = Math.min(last,frame + progress + Math.max(0,elapsedMs) * speed / 250);
  const next = Math.floor(position);
  return {frame:next,progress:position - next};
}
