// Bounded translations for the dApp signing flow. Each row is en, ko, ja,
// zh-Hans, zh-Hant, so a newly added signing string cannot silently fall back
// to English in one of the five supported wallet languages.
const rows = {
  simulationUnavailable: ['The node could not simulate this transaction. Refresh the preview before signing.', '노드에서 거래를 시뮬레이션하지 못했습니다. 서명 전에 미리보기를 새로고침하세요.', 'ノードで取引をシミュレーションできません。署名前にプレビューを更新してください。', '节点无法模拟此交易。请在签名前刷新预览。', '節點無法模擬此交易。請在簽名前重新整理預覽。'],
  requestChanged: ['The account, network, or site permission changed. Ask the site to send a new request.', '계정, 네트워크 또는 사이트 권한이 변경되었습니다. 사이트에서 새 요청을 보내야 합니다.', 'アカウント、ネットワーク、またはサイトの許可が変わりました。サイトから新しいリクエストを送ってください。', '账户、网络或网站权限已更改。请让网站发送新的请求。', '帳戶、網路或網站權限已變更。請讓網站傳送新的請求。'],
  notConnected: ['This site is no longer connected to this account.', '이 사이트는 더 이상 이 계정에 연결되어 있지 않습니다.', 'このサイトはこのアカウントに接続されていません。', '此网站已不再连接到此账户。', '此網站已不再連線至此帳戶。'],
  wrongAccount: ['The signing account does not match the connected account.', '서명 계정이 연결된 계정과 일치하지 않습니다.', '署名アカウントが接続中のアカウントと一致しません。', '签名账户与已连接账户不一致。', '簽名帳戶與已連線帳戶不一致。'],
  wrongChain: ['The message chain ID does not match the wallet network.', '메시지의 체인 ID가 지갑 네트워크와 일치하지 않습니다.', 'メッセージのチェーンIDがウォレットのネットワークと一致しません。', '消息的链 ID 与钱包网络不一致。', '訊息的鏈 ID 與錢包網路不一致。'],
  malformedTyped: ['This message has invalid or unreadable fields. Ask the site to correct it.', '메시지에 잘못되었거나 읽을 수 없는 필드가 있습니다. 사이트에서 수정해야 합니다.', 'メッセージの項目が無効か読み取れません。サイトで修正してください。', '此消息含有无效或无法读取的字段。请让网站修正。', '此訊息含有無效或無法讀取的欄位。請讓網站修正。'],
  typedTooLarge: ['This message is too large to review safely.', '메시지가 너무 커서 내용을 안전하게 검토할 수 없습니다.', 'メッセージが大きすぎて安全に確認できません。', '此消息过大，无法安全审核。', '此訊息過大，無法安全審閱。'],
  typedUnsupported: ['This account cannot sign typed messages yet. Update its account delegation in the wallet after the network deploys account v2.', '이 계정은 아직 구조화된 메시지에 서명할 수 없습니다. 네트워크에 계정 v2가 배포된 후 지갑에서 계정 위임을 업데이트하세요.', 'このアカウントはまだ型付きメッセージに署名できません。ネットワークにアカウントv2が導入された後、ウォレットで委任先を更新してください。', '此账户尚不能签署类型化消息。网络部署账户 v2 后，请在钱包中更新账户委托。', '此帳戶尚不能簽署型別化訊息。網路部署帳戶 v2 後，請在錢包中更新帳戶委派。'],
  confirmRevert: ['Confirm separately that you want to sign a transaction expected to fail and pay its fee.', '실패가 예상되는 거래에 서명하고 수수료를 지불할 것인지 별도로 확인하세요.', '失敗が予想される取引に署名し、手数料を支払うことを別途確認してください。', '请单独确认您要签署预计失败的交易并支付费用。', '請另行確認您要簽署預計失敗的交易並支付費用。'],
  previewChanged: ['The simulated result changed. Refresh and review the new preview before signing.', '시뮬레이션 결과가 변경되었습니다. 새 미리보기를 새로고침하고 확인한 후 서명하세요.', 'シミュレーション結果が変わりました。新しいプレビューを更新して確認してから署名してください。', '模拟结果已更改。请刷新并审核新预览后再签名。', '模擬結果已變更。請重新整理並審閱新預覽後再簽名。'],
  connectRequest: ['This site asks to see your address', '이 사이트에서 주소 확인을 요청합니다', 'このサイトがアドレスの閲覧を求めています', '此网站请求查看您的地址', '此網站請求查看您的地址'],
  sendRequest: ['This site asks you to send a transaction', '이 사이트에서 거래 전송을 요청합니다', 'このサイトが取引の送信を求めています', '此网站请求您发送交易', '此網站請求您傳送交易'],
  typedRequest: ['This site asks you to sign a message', '이 사이트에서 메시지 서명을 요청합니다', 'このサイトがメッセージへの署名を求めています', '此网站请求您签署消息', '此網站請求您簽署訊息'],
  connect: ['Connect', '연결', '接続', '连接', '連線'],
  approve: ['Approve transaction', '거래 승인', '取引を承認', '批准交易', '核准交易'],
  signMessage: ['Sign message', '메시지 서명', 'メッセージに署名', '签署消息', '簽署訊息'],
  reject: ['Reject', '거절', '拒否', '拒绝', '拒絕'],
  expired: ['This request was answered or has expired.', '이미 응답했거나 만료된 요청입니다.', 'このリクエストは回答済みか期限切れです。', '此请求已处理或已过期。', '此請求已處理或已過期。'],
  close: ['Close', '닫기', '閉じる', '关闭', '關閉'],
  simulationTitle: ['Transaction preview', '거래 미리보기', '取引プレビュー', '交易预览', '交易預覽'],
  simulationRunning: ['Trying the transaction on the node…', '노드에서 거래를 시뮬레이션하는 중…', 'ノードで取引を試しています…', '正在节点上模拟交易…', '正在節點上模擬交易…'],
  simulationPassed: ['The transaction is expected to succeed.', '거래가 성공할 것으로 예상됩니다.', '取引は成功すると予想されます。', '此交易预计成功。', '此交易預計成功。'],
  simulationReverted: ['The transaction is expected to fail.', '거래가 실패할 것으로 예상됩니다.', '取引は失敗すると予想されます。', '此交易预计失败。', '此交易預計失敗。'],
  failureReason: ['Reason', '실패 이유', '理由', '原因', '原因'],
  retryPreview: ['Refresh preview', '미리보기 새로고침', 'プレビューを更新', '刷新预览', '重新整理預覽'],
  simulationNotice: ['A simulation is an estimate. Chain state may change before inclusion; the network fee is separate.', '시뮬레이션은 예상 결과입니다. 거래가 포함되기 전에 체인 상태가 바뀔 수 있으며 네트워크 수수료는 별도입니다.', 'シミュレーションは予測です。取引が含まれる前にチェーンの状態が変わる場合があり、手数料は別です。', '模拟仅为预估。交易被纳入区块前链状态可能改变；网络费另计。', '模擬僅為預估。交易被納入區塊前鏈狀態可能改變；網路費另計。'],
  effectsNotice: ['Changes shown are for this account. Other contract behavior may not appear here.', '이 계정의 변경 사항을 표시합니다. 다른 컨트랙트 동작은 여기에 나타나지 않을 수 있습니다.', 'このアカウントの変化を表示します。その他のコントラクトの動作は表示されない場合があります。', '此处显示本账户的变化。其他合约行为可能不会显示。', '此處顯示本帳戶的變化。其他合約行為可能不會顯示。'],
  contractCalled: ['Contract or recipient', '컨트랙트 또는 수신인', 'コントラクトまたは送信先', '合约或收款方', '合約或收款方'],
  contractCreation: ['Creates a new contract', '새 컨트랙트 생성', '新しいコントラクトを作成', '创建新合约', '建立新合約'],
  balanceChanges: ['Expected balance changes', '예상 잔액 변동', '予想される残高の変化', '预计余额变化', '預計餘額變化'],
  noBalanceChanges: ['No balance changes were detected.', '잔액 변동이 감지되지 않았습니다.', '残高の変化は検出されませんでした。', '未检测到余额变化。', '未偵測到餘額變化。'],
  approvalsGranted: ['Spending permissions', '사용 권한', '利用の許可', '支出授权', '支出授權'],
  noApprovals: ['No new spending permissions were detected.', '새 사용 권한이 감지되지 않았습니다.', '新しい利用の許可は検出されませんでした。', '未检测到新的支出授权。', '未偵測到新的支出授權。'],
  spender: ['Spender', '사용자 주소', '利用者', '支出方', '支出方'],
  token: ['Token contract', '토큰 컨트랙트', 'トークンコントラクト', '代币合约', '代幣合約'],
  amount: ['Amount', '수량', '数量', '数量', '數量'],
  baseUnits: ['{amount} base units', '{amount} 기본 단위', '{amount} 最小単位', '{amount} 最小单位', '{amount} 最小單位'],
  unverifiedUnits: ['Token units are unverified', '토큰 단위 미검증', 'トークンの単位は未検証です', '代币单位未经验证', '代幣單位未經驗證'],
  reportedByContract: ['Movement reported by the contract; balance change was not verified', '컨트랙트가 보고한 이동이며 잔액 변동은 확인되지 않았습니다', 'コントラクトが報告した移動です。残高の変化は確認されていません', '合约报告的转移；余额变化未验证', '合約報告的移轉；餘額變化未驗證'],
  incompleteTokenCoverage: ['Some token balances could not be measured. Contract-reported movements are labeled separately.', '일부 토큰 잔액을 측정할 수 없습니다. 컨트랙트가 보고한 이동은 별도로 표시됩니다.', '一部のトークン残高は測定できませんでした。コントラクトが報告した移動には注記があります。', '部分代币余额无法测量。合约报告的转移会单独标注。', '部分代幣餘額無法測量。合約報告的移轉會另外標示。'],
  allowanceUnlimited: ['Unlimited allowance', '무제한 사용 권한', '無制限の利用許可', '无限额度', '無限額度'],
  approvalRevoke: ['Removes the spending permission', '사용 권한 해제', '利用の許可を取り消す', '撤销支出授权', '撤銷支出授權'],
  approvalAll: ['May move every NFT in this collection', '이 컬렉션의 모든 NFT를 이동할 수 있음', 'このコレクションのすべてのNFTを移動できます', '可转移此集合中的所有 NFT', '可移轉此集合中的所有 NFT'],
  approvalNFT: ['May move NFT #{id}', 'NFT #{id} 이동 허용', 'NFT #{id}を移動できます', '可转移 NFT #{id}', '可移轉 NFT #{id}'],
  nftLabel: ['NFT #{id}', 'NFT #{id}', 'NFT #{id}', 'NFT #{id}', 'NFT #{id}'],
  revertAck: ['I understand this transaction is expected to fail and may still charge a network fee.', '이 거래가 실패할 것으로 예상되며 네트워크 수수료가 부과될 수 있음을 이해합니다.', 'この取引は失敗すると予想され、手数料がかかる場合があることを理解しています。', '我理解此交易预计失败，仍可能收取网络费。', '我理解此交易預計失敗，仍可能收取網路費。'],
  networkFee: ['Network fee', '네트워크 수수료', 'ネットワーク手数料', '网络费', '網路費'],
  maxFee: ['Up to {amount} {symbol}', '최대 {amount} {symbol}', '最大 {amount} {symbol}', '最多 {amount} {symbol}', '最多 {amount} {symbol}'],
  from: ['From', '발신 계정', '送信元', '发送账户', '傳送帳戶'],
  domainName: ['Domain name', '도메인 이름', 'ドメイン名', '域名称', '網域名稱'],
  chainId: ['Chain ID', '체인 ID', 'チェーンID', '链 ID', '鏈 ID'],
  verifyingContract: ['Declared contract', '명시된 컨트랙트', '指定されたコントラクト', '声明的合约', '宣告的合約'],
  domainVersion: ['Version', '버전', 'バージョン', '版本', '版本'],
  noVerifyingContract: ['No contract is specified in this domain.', '이 도메인에 컨트랙트가 명시되지 않았습니다.', 'このドメインにはコントラクトが指定されていません。', '此域未指定合约。', '此網域未指定合約。'],
  domainNotice: ['The site supplies this domain name. Check the full contract address and every field before signing.', '이 도메인 이름은 사이트에서 제공합니다. 서명 전에 전체 컨트랙트 주소와 모든 필드를 확인하세요.', 'このドメイン名はサイトが指定しています。署名前に完全なコントラクトアドレスとすべての項目を確認してください。', '此域名称由网站提供。签名前请核对完整合约地址及所有字段。', '此網域名稱由網站提供。簽名前請核對完整合約地址及所有欄位。'],
  messageFields: ['Message fields', '메시지 필드', 'メッセージの項目', '消息字段', '訊息欄位'],
  typedWarning: ['A message signature can grant permission or authorize a future action without sending a transaction now.', '메시지 서명은 지금 거래를 보내지 않아도 권한을 부여하거나 이후 작업을 승인할 수 있습니다.', 'メッセージへの署名は、今取引を送信しなくても、許可の付与や将来の操作の承認に使われます。', '消息签名可授予权限或授权未来操作，即使现在不发送交易。', '訊息簽名可授予權限或授權未來操作，即使現在不傳送交易。'],
  signatureSigned: ['Message signed.', '메시지에 서명했습니다.', 'メッセージに署名しました。', '消息已签署。', '訊息已簽署。'],
  transactionSent: ['Transaction sent.', '거래를 전송했습니다.', '取引を送信しました。', '交易已发送。', '交易已傳送。'],
  connected: ['Connected.', '연결되었습니다.', '接続しました。', '已连接。', '已連線。'],
  chainLabel: ['Chain {chain}', '체인 {chain}', 'チェーン {chain}', '链 {chain}', '鏈 {chain}'],
  callData: ['Call data', '호출 데이터', '呼び出しデータ', '调用数据', '呼叫資料'],
  byteCount: ['{count} bytes', '{count}바이트', '{count}バイト', '{count} 字节', '{count} 位元組'],
  emptyList: ['Empty list', '빈 목록', '空の一覧', '空列表', '空清單'],
  noFields: ['No fields', '필드 없음', '項目なし', '无字段', '沒有欄位'],
  yes: ['Yes', '예', 'はい', '是', '是'],
  no: ['No', '아니요', 'いいえ', '否', '否'],
  unrecognizedLogs: ['Some contract effects could not be decoded.', '일부 컨트랙트 결과를 해석할 수 없습니다.', '一部のコントラクトの効果は読み取れませんでした。', '部分合约效果无法解码。', '部分合約效果無法解碼。'],
  account: ['Account', '계정', 'アカウント', '账户', '帳戶'],
  intendedValue: ['Native value requested', '요청한 네이티브 코인 수량', '要求されたネイティブ通貨の額', '请求的原生币金额', '請求的原生幣金額'],
  unknownFailure: ['The contract reverted without a readable reason.', '컨트랙트가 읽을 수 있는 이유 없이 실행을 되돌렸습니다.', 'コントラクトは読み取れる理由なしで処理を取り消しました。', '合约回滚，未提供可读原因。', '合約回復，未提供可讀原因。'],
  customError: ['The contract rejected the transaction with a custom error.', '컨트랙트가 사용자 정의 오류로 거래를 거절했습니다.', 'コントラクトが独自のエラーで取引を拒否しました。', '合约以自定义错误拒绝了交易。', '合約以自訂錯誤拒絕了交易。'],
  panicFailure: ['The contract stopped because of an internal error.', '컨트랙트가 내부 오류로 중단되었습니다.', 'コントラクトは内部エラーで停止しました。', '合约因内部错误停止。', '合約因內部錯誤停止。'],
};

export const STRINGS = Object.freeze(Object.fromEntries(['en', 'ko', 'ja', 'zh-Hans', 'zh-Hant'].map((locale, index) => [locale, Object.freeze(Object.fromEntries(Object.entries(rows).map(([key, values]) => [key, values[index]])))])));

export function language(value = globalThis.navigator?.language || 'en') {
  const locale = String(value).toLowerCase();
  if (locale.startsWith('ko')) return 'ko';
  if (locale.startsWith('ja')) return 'ja';
  if (locale.startsWith('zh')) return /hant|(?:^|-)tw(?:-|$)|(?:^|-)hk(?:-|$)|(?:^|-)mo(?:-|$)/.test(locale) ? 'zh-Hant' : 'zh-Hans';
  return 'en';
}

export function t(key, values = {}, locale = language()) {
  const text = STRINGS[locale]?.[key] || STRINGS.en[key];
  if (!text) throw new Error(`Unknown signing string: ${key}`);
  return text.replace(/\{(\w+)\}/g, (_, name) => String(values[name] ?? ''));
}
