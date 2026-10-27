# Vinci

**Instalador não oficial do DaVinci Resolve para qualquer distribuição Linux.**

> ⚠️ **Projeto não oficial.** O Vinci não tem nenhuma afiliação com a Blackmagic Design.
> "DaVinci Resolve" é marca registrada da Blackmagic Design Pty Ltd. O Vinci não distribui
> o Resolve: você baixa o instalador oficial no site da Blackmagic.

O DaVinci Resolve só suporta oficialmente o Rocky Linux. O Vinci roda o Resolve num
container Ubuntu 24.04 (via [distrobox](https://distrobox.it) e podman) com acesso direto
à sua GPU, então ele funciona igual em Ubuntu, Fedora, Arch, openSUSE, Mint, Pop!_OS,
Bazzite e outras, sem mexer nas bibliotecas do seu sistema.

## O que o Vinci faz

- **Verifica o computador** antes de tudo: GPU, driver, memória e espaço em disco, e
  explica em português claro o que pode dar problema.
- **Encontra o instalador** na sua pasta Downloads (ou abre o site da Blackmagic e espera
  o download terminar).
- **Instala as ferramentas necessárias** (podman e distrobox), pedindo sua confirmação.
- **Instala o Resolve** com suporte à sua GPU:

  | GPU | Como | Situação |
  |---|---|---|
  | NVIDIA | driver proprietário do sistema (CUDA) | ✅ testado |
  | AMD | ROCm dentro do container | experimental em RDNA1/Vega |
  | Intel | runtime NEO (Arc e 12ª geração+) | experimental (a Blackmagic não suporta Intel no Linux) |

- **Cria o atalho** no menu, com ícone e abertura de arquivos `.drp`/`.drt`.
- **Converte vídeos** que o Resolve não abre no Linux (H.264/H.265 na versão Free, áudio
  AAC em todas) e converte suas renderizações para MP4 *(em desenvolvimento)*.
- **Não coleta nenhum dado.** Zero telemetria.

## Status

Em desenvolvimento. O núcleo de instalação foi validado de ponta a ponta com o
Resolve 21.1.2 numa NVIDIA RTX A2000 (veja [docs/PROVA.md](docs/PROVA.md)).

## Desenvolvimento

Requisitos: Rust estável, GTK 4.14+ e libadwaita 1.5+ (`libgtk-4-dev libadwaita-1-dev`).

```sh
cargo test                     # toda a suíte
cargo run -p vinci             # abre o app
cargo run -p rli-core --example watchdog   # confere se as dependências externas ainda existem
```

- `crates/core`: toda a lógica, sem GTK, testável.
- `crates/app`: interface (relm4 + libadwaita).
- `data/recipe.toml`: tudo o que muda com o tempo (imagem, pacotes, repositórios, URLs).
- `docs/SPEC.md`: decisões de design. `docs/FACTS.md`: fatos verificados em fontes primárias.

Traduções ficam em `po/`. Um teste falha se algum texto da interface estiver sem tradução;
rode `scripts/update-po.sh` depois de mudar textos.

## Licença

GPL-3.0-or-later. Veja [LICENSE](LICENSE).

---

## English

**Unofficial DaVinci Resolve installer for any Linux distribution.** Not affiliated with
Blackmagic Design. Vinci runs Resolve in an Ubuntu 24.04 distrobox container with direct GPU
access (NVIDIA, AMD via ROCm, Intel via NEO), creates the menu entry, and helps with the codecs
Resolve can't handle on Linux. It never redistributes Resolve and collects no data.
Licensed GPL-3.0-or-later.
