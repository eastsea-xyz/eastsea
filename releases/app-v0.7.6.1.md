# EastSea 0.7.6.1 (testnet)

EastSea 0.7.6.1 for macOS (Apple silicon), signed by Pipln and notarized by Apple.

Drag the app to Applications. The app is a wallet and, with the switch on, a node that verifies every block on this Mac. It connects to the EastSea testnet (test tokens have no value).

Updates arrive automatically (EastSea ▸ Check for Updates…).

What's new:
- AI agent payments can now be set up in the app. You create the agent's key, choose how much DBLN it may spend, for how long and who may receive payments, and approve the first recipient with Touch ID. Until you approve a recipient, the agent cannot pay anyone. The app shows what it spent, the receipts and what remains of its allowance, and you can stop it at any time. Connecting the agent to the agent tools you have installed is guided in the app too.
- sea:// names can now be registered and managed in the app: your names with their expiry, their records, transfers, and renewal for up to 10 years with a single approval. Names that are close to expiry can be renewed together.
- Before a name expires you get a reminder (starting 30 days ahead), and the menu bar shows how many names you hold and when the next one expires.
- If you want, renewal happens by itself: with Touch ID you approve the names, a yearly budget and an end date, and this Mac renews expiring names on its own within that budget, even while the app is closed. You can stop it at any time and the unspent budget goes back to your wallet. Each renewal you approve runs as its own small background job, named sea.names.…, so stopping one leaves the others as they are.
- The numbers on the Network screen are now counted from the network's own record: wallets, registered Macs, Macs that answered recently, and the validators that keep the network. Before, only the momentary count of visible connections was shown. Only people's wallets are counted, not the network's own accounts, and while the node is still counting the app says so instead of showing a wrong number. The explorer's home page shows the same counted numbers.
- The globe's headline now counts Macs. The network's own operator keys were counted as if they were Macs; now they are left out of the headline and shown on the line below it, as "Validators · operator keys". If the node you are connected to is older and cannot tell them apart, the globe says how many nodes are connected instead of calling them Macs.
- The window no longer freezes for a moment while the app works. Reading balances, working out a transfer's fee, signing with Touch ID, following a sent transfer and reading rewards now happen away from the window, so typing, scrolling and the globe keep responding.
- The globe, both the card on the dashboard and the small one in the menu bar, is now drawn by your Mac's graphics chip. It turns more smoothly and more lightly, the page scrolls when the pointer is over the globe, and turning the globe by hand still works if you have reduced motion turned on.
- After an update, the app no longer warns about "a different node program" when the leftover background node is simply its own older release; it recognizes it and restarts it by itself. It takes over a background node only when the installed app is really newer.
- One bar now runs across the whole top of the window, above the sidebar and the page. The window buttons, back and forward, the Browse address field, the node's state and the account menu stay in the same place on every screen, also when the sidebar is hidden.
- That whole top bar moves the window, as in other Mac apps, and double-clicking an empty spot follows your macOS setting. Dragging uses the left mouse button, and opening the wallet from the menu bar closes the menu bar panel.
- The node's status at the bottom of the sidebar is always visible again; in a short window it was cut off.
