// Routes contain only view state. Saved runs and replays stay on the server.
// Player indices cover the 16 teams of the match server; each screen clamps them to its own match.
const MAX_PLAYERS = 16;
export function routeHash(route) {
  const params = new URLSearchParams();
  if (route.kind === 'run') {
    params.set('run',route.id);
    if (route.view !== 'results') params.set('view',route.view);
    if (route.seed != null) params.set('seed',route.seed);
    const filters = route.filters || {};
    for (const [key,value] of Object.entries(filters)) if (value && !(key === 'sort' && value === 'seed')) params.set(key,value);
  } else if (route.kind === 'compare') {
    params.set('compare',route.ids.join(','));
    if (route.baseline) params.set('baseline',route.baseline);
    if (route.metric && route.metric !== 'uniqueBrands') params.set('metric',route.metric);
    if (route.format) params.set('format',route.format);
    if (route.filter && route.filter !== 'all') params.set('cases',route.filter);
  } else if (route.kind === 'preview') {
    params.set('preview',route.seed);
    params.set('format',route.format || 'auto');
    if (route.arenaMaps === false) params.set('arenaMaps','false');
  } else if (route.kind === 'import') params.set('import',route.id);
  if (Number.isInteger(route.player) && route.player>=0 && route.player<MAX_PLAYERS) params.set('player',String(route.player));
  if (route.showOthers === false) params.set('others','0');
  if (['both','patrol','supply'].includes(route.heatmapMode)) params.set('heatmap',route.heatmapMode);
  if (route.heatmapScope === 'all') params.set('heatmapScope','all');
  if (route.heatmapTeams === 'all') params.set('heatmapTeams','all');
  if (route.frame > 0) params.set('frame',String(route.frame));
  return params.size ? `#${params}` : '';
}

export function parseRoute(hash) {
  const p = new URLSearchParams(hash.replace(/^#/,''));
  const validId = id => !!id && /^[\d-]{1,79}$/.test(id);
  const seed = value => value != null && /^\d+$/.test(value) && BigInt(value) <= 18446744073709551615n ? BigInt(value).toString() : null;
  const choice = (key,values,fallback) => values.includes(p.get(key)) ? p.get(key) : fallback;
  const format = choice('format',['first','semifinal','final'],'');
  const matchFields = {};
  if (/^\d{1,2}$/.test(p.get('player')||'') && Number(p.get('player'))<MAX_PLAYERS) matchFields.player=Number(p.get('player'));
  if(p.get('others')==='0')matchFields.showOthers=false;
  if (['both','patrol','supply'].includes(p.get('heatmap'))) matchFields.heatmapMode = p.get('heatmap');
  if (p.get('heatmapScope') === 'all') matchFields.heatmapScope = 'all';
  if (p.get('heatmapTeams') === 'all') matchFields.heatmapTeams = 'all';
  const frame = Math.max(0,Math.min(2569,Number(p.get('frame')) || 0));
  if (p.has('run') && validId(p.get('run'))) return {
    ...matchFields,kind:'run',id:p.get('run'),view:choice('view',['results','map','logs'],'results'),seed:seed(p.get('seed')),frame,
    filters:{format,status:choice('status',['AC','WA','TLE','RE'],''),search:p.get('search') || '',sort:choice('sort',['seed','score','rank','issues'],'seed')}
  };
  if (p.has('compare')) {
    const ids = [...new Set(p.get('compare').split(','))];
    if (ids.length >= 2 && ids.length <= 4 && ids.every(validId)) return {
      kind:'compare',ids,baseline:ids.includes(p.get('baseline')) ? p.get('baseline') : ids[0],format,
      metric:choice('metric',['uniqueBrands','dailyBrands','udon','answerTimeMs'],'uniqueBrands'),filter:choice('cases',['all','matched','changed','unmatched'],'all')
    };
  }
  if (seed(p.get('preview')) != null) return {...matchFields,...(p.get('arenaMaps') === 'false' ? {arenaMaps:false} : {}),kind:'preview',seed:seed(p.get('preview')),format:format || 'auto',frame};
  if (/^[\w-]{1,80}$/.test(p.get('import') || '')) return {...matchFields,kind:'import',id:p.get('import'),frame};
  return null;
}

export function withoutDeletedRuns(route,deleted,fallbackId) {
  const fallback = () => fallbackId ? {kind:'run',id:fallbackId,view:'results'} : {kind:'home'};
  if (route.kind === 'run' && deleted.has(route.id)) return fallback();
  if (route.kind === 'compare' && route.ids.some(id => deleted.has(id))) {
    const ids = route.ids.filter(id => !deleted.has(id));
    if (ids.length < 2) return ids.length ? {kind:'run',id:ids[0],view:'results'} : fallback();
    return {...route,ids,baseline:ids.includes(route.baseline) ? route.baseline : ids[0]};
  }
  return route;
}

export class Navigation {
  constructor({window,capture,render,onError,persist = () => {},normalize = route => route}) {
    Object.assign(this,{window,capture,render,onError,persist,normalize});
    this.serial = 0;
    this.busy = false;
    window.history.scrollRestoration = 'manual';
    window.addEventListener('popstate',event => {
      const saved = event.state?.testerNavigation === 1 ? event.state.route : null;
      const route = saved && routeHash(saved) === window.location.hash ? saved : parseRoute(window.location.hash);
      return this.go(route || {kind:'home'},{restore:true}).catch(onError);
    });
  }
  write(route,replace) {
    this.window.history[replace ? 'replaceState' : 'pushState']({testerNavigation:1,route},'',`${this.window.location.pathname}${this.window.location.search}${routeHash(route)}`);
    this.persist(route);
  }
  update() {
    if (this.busy || !this.route) return;
    this.route = this.capture(this.route);
    this.write(this.route,true);
  }
  async go(route,{replace = false,restore = false} = {}) {
    if (!restore) this.update();
    route = this.normalize(route);
    const serial = ++this.serial;
    const current = () => serial === this.serial;
    this.busy = true;
    this.route = route;
    if (!restore) this.write(route,replace || routeHash(route) === this.window.location.hash);
    let rendered = false;
    try {
      await this.render(route,current);
      if (!current()) return;
      rendered = true;
      this.window.scrollTo(0,route.scroll || 0);
    } finally {
      if (current()) { this.busy = false; if (rendered) this.update(); }
    }
  }
}
