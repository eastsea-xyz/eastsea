// The network-pause tracker (src/lib/pause.js), the same rule as the app's
// WalletModel.trackChainProgress.
import test from 'node:test';
import assert from 'node:assert/strict';
import { nextPauseState, pausedLine, PAUSE_AFTER_MS, STILL_GRACE_MS } from '../src/lib/pause.js';

test('constants match the app', () => {
  assert.equal(PAUSE_AFTER_MS, 60_000);
  assert.equal(STILL_GRACE_MS, 20_000);
});

test('a running chain never pauses', () => {
  let s = null;
  let t = 0;
  let h = 100;
  for (let i = 0; i < 10; i++) {
    t += 15_000;
    h += 1;
    s = nextPauseState(s, { now: t, height: h, blockAt: t - 1_000 });
    assert.equal(s.pausedSince, null, `tick ${i}`);
  }
});

test('the height alone, still for over 60 s, pauses', () => {
  let s = nextPauseState(null, { now: 0, height: 100, blockAt: 0 });
  s = nextPauseState(s, { now: 61_000, height: 100, blockAt: 55_000 });
  assert.notEqual(s.pausedSince, null);
  assert.equal(s.pausedSince, 0, 'since the height last changed');
});

test('an old newest block alone does not pause: a wrong clock here never looks like a pause', () => {
  let s = nextPauseState(null, { now: 0, height: 100, blockAt: -10 * 60_000 });
  s = nextPauseState(s, { now: 15_000, height: 100, blockAt: -9 * 60_000 + 15_000 });
  assert.equal(s.pausedSince, null);
});

test('an old block plus a still height pauses with the block time', () => {
  let s = nextPauseState(null, { now: 0, height: 100, blockAt: 0 });
  s = nextPauseState(s, { now: 21_000, height: 100, blockAt: -2 * 60_000 });
  assert.equal(s.pausedSince, -2 * 60_000, 'the older of the two');
});

test('a new block clears the pause', () => {
  let s = nextPauseState(null, { now: 0, height: 100, blockAt: 0 });
  s = nextPauseState(s, { now: 61_000, height: 100, blockAt: 55_000 });
  assert.notEqual(s.pausedSince, null);
  s = nextPauseState(s, { now: 62_000, height: 101, blockAt: 61_500 });
  assert.equal(s.pausedSince, null);
  assert.equal(s.lastHeight, 101);
});

test('a node that stops answering keeps the last state', () => {
  let s = nextPauseState(null, { now: 0, height: 100, blockAt: 0 });
  s = nextPauseState(s, { now: 61_000, height: 100, blockAt: 55_000 });
  assert.notEqual(s.pausedSince, null);
  const kept = nextPauseState(s, { now: 90_000, height: null, blockAt: null });
  assert.equal(kept.pausedSince, s.pausedSince);
  assert.equal(kept.lastHeight, 100);
});

test('the paused line reads like the app', () => {
  assert.equal(pausedLine(0, 90_000), 'Network paused · last block 1 min ago');
  assert.equal(pausedLine(0, 3 * 60_000), 'Network paused · last block 3 min ago');
  assert.equal(pausedLine(0, 125 * 60_000), 'Network paused · last block 2 h ago');
  assert.equal(pausedLine(0, 30_000), 'Network paused · last block 1 min ago', 'never "0 min"');
});
