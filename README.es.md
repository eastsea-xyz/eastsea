# EastSea Node

[English](README.md) · [한국어](README.ko.md) · [中文](README.zh-CN.md) · [日本語](README.ja.md) · [Tiếng Việt](README.vi.md) · **Español**

> Cualquier Mac puede ser un validador, y tu Mac verifica tu billetera por sí mismo. Hoy, validadores en distintas conexiones de internet domésticas llegan a consenso a través de rutas públicas, y las apps de billetera para Mac y iPhone funcionan sobre esa red.

[![Legal Disclaimer](https://img.shields.io/badge/Legal-Disclaimer%20%26%20Terms-red.svg)](DISCLAIMER.md)

> [!IMPORTANT]
> EastSea Node es software de investigación experimental y no comercial, que se ofrece **"TAL CUAL" ("AS IS")**. No es una blockchain de producción y no ha sido auditado. Todos los tokens (DBLN) y las recompensas son artefactos de prueba **sin ningún valor monetario**. Consulta [DISCLAIMER.md](DISCLAIMER.md).

## Qué es

- **Pensado primero para Mac.**
  - Las claves de la billetera viven en el Secure Enclave y cada pago pide Touch ID. No hay frase semilla.
  - Se puede registrar un segundo dispositivo Apple como clave de recuperación.
- **Verifica, no confíes.**
  - La billetera comprueba cada saldo en el dispositivo: una firma BLS de umbral del comité de validadores, más una prueba de estado EIP-7864.
  - Nunca se fía de lo que diga un servidor.
- **Sin puertos, sin VPN.**
  - Validadores y billeteras se encuentran entre sí por ID de nodo en la BitTorrent Mainline DHT.
  - Se conectan por iroh QUIC con hole punching y, si eso falla, recurren a un relay.
- **Hecho también para agentes de IA.**
  - `aether-agent` le da una billetera a Claude Code, Codex, Antigravity, OpenClaw, Hermes o cualquier cliente MCP.
  - Su clave está en el Secure Enclave, y el contrato de la cuenta aplica sus límites de gasto on-chain; solo tú puedes cambiarlos, con Touch ID.

## Pruébalo

```bash
scripts/demo.sh          # inicia 4 validadores; muestra transferencias, un contrato, verificación de pruebas y nodos llegando a acuerdo
scripts/devnet.sh stop   # los detiene
```

Requiere el toolchain de rustup 1.98.1 (`rust-toolchain.toml`). Si el rustc de Homebrew aparece primero en tu PATH, ejecuta `export PATH="$HOME/.cargo/bin:$PATH"`.

### App de billetera (macOS, iOS)

```bash
scripts/build-wallet.sh           # app para macOS
scripts/build-wallet.sh ios-sim   # simulador de iOS
```

- **Modo Simple (predeterminado):**
  - Una pantalla de inicio con el saldo y un gráfico del saldo, además de Enviar, Recibir (QR) y Obtener tokens de prueba.
  - Páginas de actividad, estado de la red y configuración de recuperación.
- **Modo Developer:** pruebas, raíces de estado, logs y bloques sin procesar. Usa el interruptor de la esquina superior izquierda para cambiar de modo.

### Billetera para agentes de IA

```bash
scripts/build-agent.sh --install   # ~/.local/bin/aether-agent
aether-agent init                  # tú, una sola vez: crea las claves; financia la cuenta y luego fija los límites (Touch ID)
aether-agent setup all --apply     # registra el servidor MCP "aether" en todas las herramientas de agentes que tengas instaladas
```

- **Herramientas:** status, wallet, balance, send, pay_many (una sola transacción), receipt, history.
- **Límites predeterminados:** 1 DBLN por pago y 10 DBLN cada 24 horas. Cámbialos con `aether-agent policy set`, que pide Touch ID.
- **Aplicado on-chain:** el agente paga con una clave de sesión de su cuenta; el contrato comprueba cada pago frente a los límites, los destinatarios permitidos y la caducidad. Ningún archivo ni proceso local puede saltárselos. El gas sale de un pequeño saldo aparte.
- Detalles: [AGENTS.md](AGENTS.md) y [el archivo de skill](agents/skills/aether-wallet/SKILL.md).

### Ejecutar una red real

En cada máquina validadora, genera su propia clave. Luego reúnan las mitades públicas, realicen juntos la ceremonia de claves e inicien los nodos:

```bash
aether keygen --data ~/aether/v1                         # en cada máquina; el secreto se queda ahí
aether network v1.pub.json v2.pub.json … > network.json  # solo las mitades públicas; compártelo con todos
AETHER_NETWORK=network.json scripts/devnet.sh dkg 4      # otras máquinas: aether dkg --network network.json --port … --data …
AETHER_NETWORK=network.json scripts/devnet.sh start 4    # otras máquinas: aether node --network network.json …
# billetera: copia <data>/network.json (IDs de nodo + clave del comité) a apps/wallet/Resources/
```

La clave del comité se mantiene aunque cambien los validadores. Usa `aether reshare` para pasar a un nuevo conjunto de validadores.

### Línea de comandos

```bash
target/debug/aether dev-accounts                               # claves de prueba públicas con fondos desde el génesis (sin valor)
target/debug/aether send --from-dev 1 --to 0x… --value 1000 --wait
target/debug/aether balance 0x… --rpc http://127.0.0.1:8547   # verificado localmente con una prueba, sin confiar
target/debug/aether batch --from-dev 1 --to 0xA,0xB --value 1  # varios pagos, una sola firma
target/debug/aether blocks 10
```

## Estado (2026-09-26)

| Área | Funciona hoy | Pendiente |
|---|---|---|
| Consenso | Commonware simplex BFT: 4 validadores, bloques de 1 s, sigue funcionando con un validador caído. Certificados de umbral BLS12-381 (131 B, verificados con una sola clave de grupo). Claves generadas localmente; clave del comité mediante un DKG sin dealer. Rotación de validadores mediante reshare. Líder aleatorio con semilla VRF | Cambios del comité on-chain, selección del comité por VRF |
| Red | iroh QUIC entre validadores y billeteras, con hole punching o un relay. Direcciones encontradas por ID de nodo en la Mainline DHT. Nunca se usan rutas de Tailscale, CGNAT ni loopback. Probado con Macs en dos ISP distintos | Lista de validadores on-chain, relays propios |
| Ejecución | revm: transferencias, despliegue y llamadas de contratos. Cada validador vuelve a ejecutar cada bloque y debe coincidir con su lista de acceso del bloque (BAL) y su gas. La ejecución paralela optimista da el mismo resultado que la secuencial | Commits paralelos del árbol |
| Comisiones | Base fees separadas para ejecución y para generación de pruebas, ajustadas como en EIP-4844. La base fee de ejecución se quema y la comisión de pruebas va a un escrow del prover. Las propinas se reparten 60% al proposer, 20% al escrow del prover y 20% se quema | Reclamos del escrow por cada chunk probado |
| Estado | Árbol binario EIP-7864 (Poseidon2), con claves y raíces que coinciden con la referencia de geth. Pruebas de inclusión y de ausencia. Se guarda de forma atómica por bloque en redb y se reanuda desde un checkpoint tras un reinicio | Paginación en disco, sincronización por snapshot |
| Cuentas | P-256 (Secure Enclave), secp256k1 y Ed25519. La delegación EIP-7702 a `AetherAccount` permite pagos agrupados con una sola firma. La clave del Secure Enclave de un segundo dispositivo puede funcionar como clave de recuperación | Claves de sesión, múltiples guardianes, recuperación con bloqueo temporal |
| Clientes | Las billeteras para Mac e iOS (modos Simple y Developer), la CLI y `aether-agent` (MCP) verifican los saldos localmente | Pruebas ZK de bloques en el cliente, TestFlight |
| Resistencia a la censura | Listas de inclusión al estilo FOCIL: los validadores se niegan a votar por un bloque que omite transacciones listadas | Mempool cifrado |
| Generación de pruebas (spike) | Jolt zkVM prueba bloques reales de EastSea, y las raíces coinciden con la ejecución nativa. Unas 270 transacciones por hora por Mac. Las pruebas van por detrás de la cadena | Backend de Metal, pruebas de checkpoint |

`legacy/` es la demo anterior de un solo nodo y ya fue reemplazada.

## Diseño

- [Diseño de la implementación](docs/design/00-overview.md): identidad, decisiones D1–D18 y el diseño de cada capa
- [Investigación](docs/research/): las fuentes detrás de cada decisión, incluida [tokenomics 2026](docs/research/tokenomics-2026.md)
- [Resultados del spike](docs/research/spike-2026-10.md)

## Pruebas

```bash
cargo test --workspace
cargo test -p aether-state --test eip7864_compat   # contrasta con la referencia de EIP-7864
cargo test -p aether-execution --test fees         # reparto de comisiones y conservación del valor
```

## Estructura

```
crates/
├── node/        # aether binary: validator (simplex + marshal), JSON-RPC, CLI, DKG
├── execution/   # revm execution, tx validation, BAL, fees, receipts, prove gas
├── state/       # EIP-7864 binary tree and proofs
├── types/       # envelopes, blocks, BAL, certificates, proofs
├── light/       # light client: committee key, certificate checks
├── net/         # iroh links, Mainline DHT discovery
├── ffi/         # wallet core for Swift (UniFFI)
├── hash/ crypto/ consensus/ proving/ da/
apps/
├── wallet/      # macOS and iOS wallet (SwiftUI)
└── agent/       # aether-agent: MCP server and JSON CLI for AI agents
agents/skills/   # SKILL.md for agent tools
contracts/       # AetherAccount (EIP-7702 batch + recovery)
spike/           # zkVM proving experiments
scripts/         # devnet.sh, demo.sh, build-wallet.sh, build-agent.sh
docs/design, docs/research
```

## Licencia

Con doble licencia: MIT o Apache-2.0.
