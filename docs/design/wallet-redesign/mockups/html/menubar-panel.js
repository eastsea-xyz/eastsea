/* Static design prototype. Buttons report their action in the board caption;
   this file never launches the wallet or connects to a node. */
const lang = document.documentElement.lang;
const copy = lang === 'ko' ? {
  connected: '연결됨 · 방금 확인했어요',
  paused: '일시 정지됨',
  copied: '프로토타입: 샘플 주소를 복사했어요.',
  copyUnavailable: '프로토타입 주소: ',
  open: '프로토타입 동작: EastSea 창 열기'
} : {
  connected: 'Connected · Checked just now',
  paused: 'Paused',
  copied: 'Prototype: sample address copied.',
  copyUnavailable: 'Prototype address: ',
  open: 'Prototype action: open the EastSea window'
};
const actionCaption = document.querySelector('.prototype-action');
const nodeSwitch = document.querySelector('.node-switch');
const nodeRow = document.querySelector('.node-row');
const statusText = document.querySelector('.status-text');

nodeSwitch.addEventListener('click', () => {
  const running = nodeSwitch.getAttribute('aria-checked') !== 'true';
  nodeSwitch.setAttribute('aria-checked', String(running));
  nodeRow.dataset.running = String(running);
  statusText.textContent = running ? copy.connected : copy.paused;
});

document.querySelector('.copy-address').addEventListener('click', async () => {
  const address = document.querySelector('.address').dataset.address;
  try {
    await navigator.clipboard.writeText(address);
    actionCaption.textContent = copy.copied;
  } catch {
    actionCaption.textContent = copy.copyUnavailable + address;
  }
});

document.querySelector('.open-wallet').addEventListener('click', () => {
  actionCaption.textContent = copy.open;
});
