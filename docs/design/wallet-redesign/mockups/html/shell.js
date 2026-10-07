// Shared Mac sidebar (spec.md §3 IA). <aside class="sidebar" data-active="home" data-lang="en"></aside>
const L = {
  en: { wallet: 'Wallet', home: 'Home', activity: 'Activity', explore: 'Explore', mac: 'This Mac', node: 'Node & rewards',
        protect: 'Protection', security: 'Security', agents: 'AI agent payments', nodeOn: 'Node on this Mac', verifying: 'Verifying blocks',
        connected: 'Connected', block: 'Checked just now' },
  ko: { wallet: '지갑', home: '홈', activity: '활동', explore: '둘러보기', mac: '이 Mac', node: '노드와 보상',
        protect: '보호', security: '보안', agents: 'AI 에이전트 결제', nodeOn: '이 Mac의 노드', verifying: '블록 확인 중',
        connected: '연결됨', block: '방금 확인함' },
};
const icon = n => `<svg><use href="#i-${n}"/></svg>`;
document.querySelectorAll('aside.sidebar[data-active]').forEach(el => {
  const t = L[el.dataset.lang || 'en'], a = el.dataset.active;
  const nav = (id, ic, extra = '') => `<div class="nav ${a === id ? 'on' : ''}">${icon(ic)}${t[id]}${extra}</div>`;
  const nodeOff = el.dataset.node === 'off', offline = el.dataset.net === 'off';
  el.innerHTML = `<div class="lights"><i></i><i></i><i></i></div>
    <div class="side-h">${t.wallet}</div>
    ${nav('home', 'home')}${nav('activity', 'activity', el.dataset.pending ? `<span class="badge">${el.dataset.pending}</span>` : '')}${nav('explore', 'explore')}
    <div class="side-h">${t.mac}</div>
    ${nav('node', 'node')}
    <div class="side-h">${t.protect}</div>
    ${nav('security', 'shield', el.dataset.secdot ? '<span class="dot"></span>' : '')}${nav('agents', 'agent', el.dataset.agentdot ? '<span class="dot"></span>' : '')}
    <div class="side-foot">
      <div class="row"><div style="flex:1"><b>${t.nodeOn}</b><span class="s">${nodeOff ? (el.dataset.lang === 'ko' ? '꺼짐' : 'Off') : t.verifying}</span></div><div class="switch ${nodeOff ? '' : 'on'}"></div></div>
      <div class="row"><span class="live ${offline ? 'warn' : ''}"></span><div><b>${offline ? (el.dataset.lang === 'ko' ? '연결 대기 중' : 'Reconnecting') : t.connected}</b><span class="s">${offline ? (el.dataset.lang === 'ko' ? '마지막 확인 4분 전' : 'Last checked 4 min ago') : t.block}</span></div></div>
    </div>`;
});
