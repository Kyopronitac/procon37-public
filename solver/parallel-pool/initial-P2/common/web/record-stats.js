export const SCORE_KEYS = ['uniqueBrands','dailyBrands','udon'];
export function scoreTotals(cases) {
  return cases.reduce((sum,c) => {
    for (const key of SCORE_KEYS) sum[key] += c.score[key];
    return sum;
  },{uniqueBrands:0,dailyBrands:0,udon:0});
}
export function scoreOrder(a,b) {
  return Math.sign(a.score.uniqueBrands - b.score.uniqueBrands || a.score.dailyBrands - b.score.dailyBrands || a.score.udon - b.score.udon);
}
export function allAccepted(row) {
  return row.alignment === 'matched' && row.cases.every(c => c?.status === 'AC');
}
export function winCounts(rows,index,baseline) {
  const result = {wins:0,draws:0,losses:0};
  for (const row of rows.filter(allAccepted)) {
    const order = scoreOrder(row.cases[index],row.cases[baseline]);
    result[order > 0 ? 'wins' : order < 0 ? 'losses' : 'draws']++;
  }
  return result;
}
