# EastSea 0.7.7 (testnet)

EastSea 0.7.7 for macOS (Apple silicon), signed by Pipln and notarized by Apple.

Drag the app to Applications. The app is a wallet and, with the switch on, a node that verifies every block on this Mac. It connects to the EastSea testnet (test tokens have no value).

Updates arrive automatically (EastSea ▸ Check for Updates…).

What's new:
- A welcome reward for registered Macs: a Mac that has been registered for a day gets a one-time welcome reward (DBLN) from the shared sponsor pool. One per Mac, up to 1,000 Macs while the pool lasts. Macs that are already registered get it too, and there is nothing to apply for.
- Seats are drawn daily: the seats of the Macs that check the network are drawn anew every day. Registered Macs that answered at least 80% of their online checks over the last day take part, and a drawn Mac moves into its seat on its own.
- A quiet network opens fewer proofs: a new proof task opens only when real transfers are moving through the network.
- Apps can be published and found: a published app shows up in search. Any change a publisher makes to an app waits two days before it takes effect — no exceptions. A publisher can have at most 16 apps in search at a time.
- Names are simpler: one screen shows your sea names, when each expires and whether it renews on its own, and auto-renew now follows the real expiry date. You can type a name wherever an address goes, to send or to share where you receive.
- Apps you open are shared like a torrent: apps you open or pin with sea:// are passed on to other Macs while this Mac's node runs, and those Macs can see this Mac's IP address. Settings ▸ App sharing ▸ "Share apps I open" turns it off.
- Registration no longer fails at random: when a block lands while the wallet reads the registration details, it reads them again instead of giving up.
- One share per Mac: from the switch-on, every registered Mac earns its own share of the hourly node reward, paid to the address that registered it. Someone who runs several Macs gets one share for each, whether they use one address or many. Each Mac at most 1/16; with more than 16 Macs they share by weight. For example, with 2 Macs online each gets 1/16; with 16 Macs each gets 1/16; with 40, each 1/40. In a small network one person with 16 Macs can take the whole hour's node share — the old rule (each operator address at most 1/16, extra Macs add nothing) allowed the same by splitting addresses. The proof reward stays at most 1/16 per address.
- A stability reward pool: no part of the node reward is left unminted any more. What the limits leave first tops up the welcome-reward pool, and the rest builds up in the stability reward pool, which nobody can withdraw from. Once all 16 validator seats are held by ordinary Macs, each time the number of Macs passes its record, a tenth of the growth (at most 10% at once) is shared by all Macs. If the number does not grow, nothing is paid out.
- Spare computers serve without reward: while a spare computer prepared by Pipln holds a checking seat, it earns nothing.
- A stopped seat is filled right away: when a checking Mac goes quiet, a spare computer takes its seat without waiting for the next daily draw. If a second Mac stops after that, the network still runs. A Mac that announced its departure leaves its seat before the next draw.
- Time to register for the four founding Macs: the four Macs that started the network keep their seats until the end of the first full draw period after the switch-on (about two to three days). From the next draw on, the same rules apply to everyone.
- If the daily check goes quiet, the network carries on: the service that re-checks each Mac every day can go silent or withdraw its checks. Registered Macs then answer and earn without it until it speaks again. New registrations still need it. A stuck registration request now expires after a day.
- Limits that protect independence apply only to a full network: the rules that keep one operator from holding many seats, and the limit on how many seats spare computers may hold, take effect only once the network has all of its seats. Before that, spares can fill every seat needed. Once the seats are full, one seat moves from the founding Macs to other Macs each day until the limits are met.
- The app tells you where the network stands: Settings ▸ This Mac's node now shows one line — "The network keeps running even if Pipln's computers stop." or "The network still needs Pipln's computers to keep running."
- Problems skip instead of stopping: if a reward settlement comes out wrong for one stretch, the network skips just that payment and keeps confirming blocks instead of stopping.
