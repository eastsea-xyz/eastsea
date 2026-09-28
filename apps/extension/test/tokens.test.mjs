// Checks the token scan against a fake chain (no node), mirroring the app's
// apps/wallet/Tests/assets/main.swift.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import {
  SEL, CAPS, parseTokenSources, emptyCatalog, discoverTokens, scanTokens,
  tokenInfo, formatTokenAmount, call, wordAddress, wordUint, uintAt, uint64At, addressAt, stringAt,
} from '../src/lib/tokens.js';

const word = (v) => wordUint(v);
const addrWord = (a) => wordAddress(a);
const str = (s) => {
  const b = Buffer.from(s, 'utf8').toString('hex');
  return `0x${word(32)}${word(b.length / 2)}${b.padEnd(Math.ceil(b.length / 64) * 64, '0')}`;
};

const owner = '0x00000000000000000000000000000000000000aa';
const factory = '0x00000000000000000000000000000000000000f1';
const pairs = '0x00000000000000000000000000000000000000f2';
const launch = '0x00000000000000000000000000000000000000f3';
const waeth = '0x00000000000000000000000000000000000000e1';
const tA = '0x00000000000000000000000000000000000000a1';
const tB = '0x00000000000000000000000000000000000000b1';
const tC = '0x00000000000000000000000000000000000000c1';
const pair = '0x00000000000000000000000000000000000000d1';
const notToken = '0x00000000000000000000000000000000000000ee';

const meta = {
  [tA]: { symbol: 'NEB', name: 'Nebula', decimals: 18 },
  [tB]: { symbol: 'ORB', name: 'Orb', decimals: 18 },
  [tC]: { symbol: 'USDX', name: 'Test Dollar', decimals: 6 },
  [waeth]: { symbol: 'WAETH', name: 'Wrapped AETH', decimals: 18 },
};
let balances = { [tA]: `0x${'0'.repeat(47)}15af1d78b58c40000`, [tB]: word(0), [tC]: word(1_500_000), [waeth]: word(0) };

function fakeChain() {
  const calls = [];
  const read = async (to, data) => {
    calls.push([to, data]);
    const sel = data.slice(2, 10);
    const arg = data.slice(10);
    if (to === factory && sel === SEL.allTokensLength) return word(2);
    if (to === factory && sel === SEL.allTokens) return addrWord(arg.endsWith('0') ? tA : notToken);
    if (to === pairs && sel === SEL.allPairsLength) return word(1);
    if (to === pairs && sel === SEL.allPairs) return addrWord(pair);
    if (to === pair && sel === SEL.token0) return addrWord(waeth);
    if (to === pair && sel === SEL.token1) return addrWord(tB);
    if (to === launch && sel === SEL.tokenCount) return word(1);
    if (to === launch && sel === SEL.tokens) return addrWord(tC);
    const m = meta[to];
    if (!m) throw new Error('bad answer from the node');
    if (sel === SEL.decimals) return word(m.decimals);
    if (sel === SEL.symbol) return str(m.symbol);
    if (sel === SEL.name) return str(m.name);
    if (sel === SEL.balanceOf) {
      assert.equal(arg, addrWord(owner), 'balanceOf owner');
      return balances[to];
    }
    throw new Error('bad answer from the node');
  };
  return { read, calls };
}

const sources = { network: 't', waeth, tokenFactory: factory, pairFactory: pairs, launchpad: launch, seed: [] };

test('scan finds tokens from every list and reports non-zero holdings', async () => {
  const { read } = fakeChain();
  const { catalog, held } = await scanTokens({ owner, sources, catalog: emptyCatalog(), read });
  assert.equal(Object.keys(catalog.tokens).length, 4);
  assert.deepEqual(catalog.rejected, [notToken]);
  assert.equal(catalog.factoryRead, 2);
  assert.equal(catalog.pairsRead, 1);
  assert.equal(catalog.launchesRead, 1);
  assert.deepEqual(held.map((x) => x.token.symbol), ['NEB', 'USDX']);
  assert.equal(held[0].balance, '25000000000000000000');
  assert.equal(formatTokenAmount(held[0].balance, held[0].token.decimals), '25');
  assert.equal(formatTokenAmount(held[1].balance, held[1].token.decimals), '1.5');
  assert.equal(held[1].token.name, 'Test Dollar');
  assert.equal(held[1].token.decimals, 6);
});

test('a second scan only reads balances, and dust shows as <0.000001', async () => {
  const { read, calls } = fakeChain();
  const first = await scanTokens({ owner, sources, catalog: emptyCatalog(), read });
  const firstCalls = calls.length;
  balances = { ...balances, [tB]: word(1) };
  const second = await scanTokens({ owner, sources, catalog: first.catalog, read });
  assert.deepEqual(second.catalog, first.catalog);
  assert.equal(calls.length - firstCalls, 3 + 4, 'the three list counts and four balances');
  assert.deepEqual(second.held.map((x) => x.token.symbol), ['NEB', 'ORB', 'USDX']);
  assert.equal(formatTokenAmount(second.held[1].balance, second.held[1].token.decimals), '<0.000001');
});

test('a failing balance aborts the scan, a failing list just stops it', async () => {
  const chain = fakeChain();
  const broken = async (to, data) => (data.slice(2, 10) === SEL.balanceOf ? Promise.reject(new Error('refused')) : chain.read(to, data));
  await assert.rejects(scanTokens({ owner, sources, catalog: emptyCatalog(), read: broken }), /refused/);
  const noList = async (to, data) => (data.slice(2, 10) === SEL.allTokensLength ? Promise.reject(new Error('refused')) : chain.read(to, data));
  const stopped = await discoverTokens(sources, emptyCatalog(), noList);
  assert.equal(stopped.factoryRead, 0, 'the factory list stops where it is');
});

test('long lists stop at the caps, and the cap holds on later scans', async () => {
  const read = async (to, data) => {
    const sel = data.slice(2, 10);
    if (to === factory && sel === SEL.allTokensLength) return word(501);
    if (to === factory && sel === SEL.allTokens) return addrWord(tA);
    throw new Error('bad answer from the node');
  };
  const capped = await discoverTokens({ tokenFactory: factory }, emptyCatalog(), read);
  assert.equal(capped.factoryRead, CAPS.factoryTokens, 'capped at 500');
  const again = await discoverTokens({ tokenFactory: factory }, capped, read);
  assert.equal(again.factoryRead, CAPS.factoryTokens, 'a capped list is not read past the cap again');
});

test('tokenInfo rejects what has no sane decimals and caps its strings', async () => {
  const nonsense = async () => '0x';
  assert.equal(await tokenInfo(tA, nonsense), null);
  const huge = async (to, data) => (data.slice(2, 10) === SEL.decimals ? word(78) : word(0));
  assert.equal(await tokenInfo(tA, huge), null);
  const long = {
    [SEL.decimals]: () => word(9),
    [SEL.symbol]: () => str('SYMBOLLONGERTHANSIXTEEN'),
    [SEL.name]: () => str('A name that is far longer than the forty-eight character cap'),
  };
  const info = await tokenInfo(tA, (to, data) => long[data.slice(2, 10)]());
  assert.deepEqual(info, { address: tA, symbol: 'SYMBOLLONGERTHAN', name: 'A name that is far longer than the forty-eight c', decimals: 9 });
  const weird = { ...long, [SEL.symbol]: () => '0x' }; // a symbol that does not decode: ???
  assert.equal((await tokenInfo(tA, (to, data) => weird[data.slice(2, 10)]())).symbol, '???');
});

test('ABI words encode and decode exactly', () => {
  assert.equal(call(SEL.balanceOf, wordAddress(owner)), `0x${SEL.balanceOf}${'0'.repeat(24)}${owner.slice(2)}`);
  assert.equal(uintAt(word(0xdeadbeef)), 0xdeadbeefn);
  assert.equal(uintAt(`0x${'f'.repeat(64)}`), 2n ** 256n - 1n);
  assert.equal(addressAt(`0x${'12'.repeat(32)}`), `0x${'12'.repeat(20)}`);
  assert.equal(stringAt(str('Wrapped AETH')), 'Wrapped AETH');
  assert.equal(stringAt(str('')), '');
  assert.throws(() => uintAt('0x1234'), /bad answer/); // not whole words
  assert.throws(() => uintAt('0xzz'.repeat(32)), /bad answer/);
});

test('uint64At accepts only words that fit a uint64', () => {
  assert.equal(uint64At(word(1_500_000)), 1_500_000);
  assert.equal(uint64At(`0x${'0'.repeat(48)}ffffffffffffffff`), Number(2n ** 64n - 1n));
  assert.throws(() => uint64At(`0x${'0'.repeat(47)}1${'0'.repeat(16)}`), /bad answer/);
});

test('formatTokenAmount mirrors the app formatting', () => {
  assert.equal(formatTokenAmount('0', 18), '0');
  assert.equal(formatTokenAmount('1234567', 0), '1234567');
  assert.equal(formatTokenAmount('1234567', -1), '1234567');
  assert.equal(formatTokenAmount('1500000000000000000', 18), '1.5');
  assert.equal(formatTokenAmount('12345678901234567890123', 18), '12345.678901');
  assert.equal(formatTokenAmount('1000000', 6), '1');
  assert.equal(formatTokenAmount(word(1).slice(2), 18), '<0.000001');
  assert.equal(formatTokenAmount(1n, 18), '<0.000001');
});

test('the bundled token-sources.json parses for chain 7780', () => {
  const file = readFileSync(join(dirname(fileURLToPath(import.meta.url)), '..', 'token-sources.json'), 'utf8');
  const sources7780 = parseTokenSources(file, 7780);
  assert.equal(sources7780.network, 'aether-testnet');
  assert.match(sources7780.waeth, /^0x[0-9a-f]{40}$/);
  assert.match(sources7780.tokenFactory, /^0x[0-9a-f]{40}$/);
  assert.match(sources7780.pairFactory, /^0x[0-9a-f]{40}$/);
  assert.match(sources7780.launchpad, /^0x[0-9a-f]{40}$/);
  assert.equal(sources7780.seed.length, 3);
  assert.equal(parseTokenSources(file, 7781), null);
  assert.equal(parseTokenSources('not json', 7780), null);
  assert.deepEqual(parseTokenSources('{"chains":{"7780":{"seed":["0x1"]}}}', 7780).seed, ['0x1']);
});
