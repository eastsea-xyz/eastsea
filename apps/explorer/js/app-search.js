// Extend the explorer's existing Operate surface: node order is the reading
// order, full publisher and sea:// URL identify each entry, and hash presence
// is separate from publisher safety. All chain strings remain text nodes.
import { h, loading, message, pill } from './dom.js';
import { resolveSearchLocale, searchText } from './search-catalog.js';

export const SEARCH_LIMIT = 50;
export const SEARCH_QUERY_BYTES = 256;

/** The same bounded RPC used by the wallet, without client ranking/filtering. */
export async function fetchAppSearch(node, query, limit = SEARCH_LIMIT) {
  if (query == null) throw new RangeError('invalidQuery');
  const text = String(query).trim();
  if (new TextEncoder().encode(text).length > SEARCH_QUERY_BYTES) throw new RangeError('queryTooLong');
  if (!text) return [];
  const value = Number(limit);
  const bounded = Number.isFinite(value) ? Math.max(1, Math.min(SEARCH_LIMIT, Math.floor(value))) : SEARCH_LIMIT;
  const results = await node.call('aether_search', [text, bounded]);
  if (!Array.isArray(results) || results.length > bounded || results.some((r) => !validRecord(r))) {
    throw new TypeError('Invalid aether_search response');
  }
  return results;
}

function validRecord(record) {
  return record && typeof record === 'object'
    && ['name', 'title', 'description', 'category', 'publisher', 'url'].every((key) => typeof record[key] === 'string')
    && typeof record.verified === 'boolean'
    && Number.isSafeInteger(record.usage_7d) && record.usage_7d >= 0
    && Number.isSafeInteger(record.created_at) && record.created_at >= 0
    && (record.lookalike == null || typeof record.lookalike === 'string');
}

function validSearchInfo(info) {
  return info && typeof info === 'object' && !Array.isArray(info)
    && ['usage_complete', 'history_complete', 'sources_configured'].every((key) => typeof info[key] === 'boolean')
    && Number.isSafeInteger(info.rejected_records) && info.rejected_records >= 0;
}

/** Chain URLs stay visible even when malformed; only sea:// is actionable. */
function seaLink(url, ...children) {
  return /^sea:\/\/[^\s\u0000-\u001f\u007f]+$/u.test(url)
    ? h('a', { href: url }, ...children)
    : h('span', {}, ...children);
}

function resultItem(record, locale, usageAvailable) {
  const t = (key, values) => searchText(locale, key, values);
  const date = new Date(record.created_at * 1000);
  const registered = Number.isFinite(date.getTime())
    ? h('time', { datetime: date.toISOString() }, date.toLocaleString(locale)) : String(record.created_at);
  const publisher = /^0x[0-9a-f]{40}$/i.test(record.publisher)
    ? h('a', { href: `#/account/${record.publisher.toLowerCase()}` }, record.publisher)
    : h('span', {}, record.publisher);
  return h('li', { class: 'app-search-result' },
    h('div', { class: 'row wrap' },
      h('h2', { class: 'app-search-title' }, seaLink(record.url, h('bdi', {}, record.title || record.name))),
      pill(t(record.verified ? 'hashPresent' : 'hashAbsent'))),
    h('p', { class: 'app-search-name mono' }, h('bdi', {}, record.name)),
    record.description ? h('p', { class: 'app-search-description' }, record.description) : null,
    h('dl', { class: 'app-search-meta' },
      h('dt', {}, t('publisher')), h('dd', { class: 'mono' }, publisher),
      h('dt', {}, t('url')), h('dd', { class: 'mono' }, seaLink(record.url, record.url)),
      record.category ? [h('dt', {}, t('category')), h('dd', {}, record.category)] : [],
      h('dt', {}, t('usage')), h('dd', {}, usageAvailable
        ? record.usage_7d.toLocaleString(locale) : t('usageUnavailable')),
      h('dt', {}, t('registered')), h('dd', {}, registered)),
    record.lookalike ? message('warn', t('lookalike', { name: record.lookalike })) : null);
}

/** A retry replaces only this page's content; the router still owns navigation. */
export async function appSearchView(ctx, query, limit = SEARCH_LIMIT) {
  const locale = resolveSearchLocale(ctx.locale || globalThis.navigator?.languages || globalThis.navigator?.language || 'en');
  const t = (key, values) => searchText(locale, key, values);
  const content = h('div', { class: 'stack', 'aria-live': 'polite' });
  const page = h('div', { class: 'stack app-search', lang: locale },
    h('h1', { class: 'page-title' }, t('search')),
    query ? h('p', { class: 'app-search-query' }, t('resultsFor', { query })) : null,
    h('p', { class: 'small muted' }, t('ranking')),
    h('p', { class: 'small muted' }, t('hashHelp')),
    content);
  async function read() {
    content.setAttribute('aria-busy', 'true');
    content.replaceChildren(loading(t('loading')));
    try {
      const results = await fetchAppSearch(ctx.node, query, limit);
      const source = ctx.node.url;
      // Older nodes can serve results without this transparency RPC. Its
      // absence is never a reason to replace or hide returned chain records.
      const status = query ? await ctx.node.call('aether_searchInfo', []).catch(() => null) : null;
      const info = validSearchInfo(status) ? status : null;
      const usageIncomplete = info?.usage_complete === false || results.some((r) => r.usage_complete === false);
      content.replaceChildren(...[
        query && !info ? message('warn', t('statusUnavailable')) : null,
        status?.sources_configured === false ? message('warn', t('sourcesUnconfigured')) : null,
        usageIncomplete ? message('warn', t('usageIncomplete')) : null,
        info?.history_complete === false ? message('warn', t('historyIncomplete')) : null,
        info?.rejected_records > 0 ? message('warn', t('indexIncomplete', { count: info.rejected_records.toLocaleString(locale) })) : null,
        results.length ? h('ol', { class: 'card app-search-results' }, ...results.map((r) => resultItem(r, locale,
          r.usage_complete === true || (info?.usage_complete === true && r.usage_complete !== false))))
          : message('plain', t(query ? 'empty' : 'prompt')),
        query ? h('p', { class: 'small muted source' }, t('source', { url: source })) : null,
      ].filter(Boolean));
    } catch (error) {
      const key = error instanceof RangeError && ['queryTooLong', 'invalidQuery'].includes(error.message) ? error.message : 'failed';
      content.replaceChildren(...[message('error', t(key)),
        key === 'failed' ? h('button', { class: 'app-search-retry', type: 'button', onclick: read }, t('retry')) : null].filter(Boolean));
    } finally {
      content.setAttribute('aria-busy', 'false');
    }
  }
  await read();
  return page;
}
