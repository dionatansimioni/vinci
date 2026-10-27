# Fatos verificados (2026-10-27)

Pesquisa em fontes primárias para validar a [especificação](SPEC.md). Datas e versões mudam; reconferir antes de cada release.

## DaVinci Resolve

- Última estável: **21.1.2 build 10** (27/10/2026), Free e Studio.
  - `GET https://www.blackmagicdesign.com/api/support/latest-stable-version/davinci-resolve/linux` → `{"linux":{"releaseId","downloadId","major","minor","releaseNum","build"}}` (Studio: `.../davinci-resolve-studio/linux`).
- Requisitos Linux (readme 21.1.2): **Rocky Linux 8.6** é a única distro listada; 32 GB RAM; GPU discreta ≥ 4 GB VRAM (IA avançada: 16 GB); OpenCL 1.2 ou CUDA 12.8; **NVIDIA ≥ 580.119.02**; AMD "drivers oficiais"; **Intel não listada para Linux**.
  - https://www.blackmagicdesign.com/support/content/readme/1dcfa6f38ebc4031a3a8561eea3aeee3

## Codecs no Linux (lista oficial 21.1)

https://documents.blackmagicdesign.com/SupportNotes/DaVinci_Resolve_21_Supported_Codec_List.pdf

| Codec | Free | Studio |
|---|---|---|
| H.264/H.265 decode | ✗ | ✓ (acelerado em NVIDIA) |
| H.264/H.265 encode | ✗ | ✓ **só com NVIDIA** |
| AAC decode/encode | ✗ | ✗ |
| AV1 | — | só NVIDIA |

- Encode ≥ 4K é exclusivo do Studio.
- SDK de plugins de encode: introduzido no Studio 17; suporte no Free não confirmado.

## API de download da Blackmagic

- O AUR **abandonou** a API (desde 18.x usa `file://` com download manual).
- Fluxo antigo: `POST https://www.blackmagicdesign.com/api/register/us/download/<downloadId>` com JSON `firstname, lastname, email, phone, country, street, state, city, product`; headers `Content-Type: application/json;charset=UTF-8`, User-Agent de navegador, `Referer: https://www.blackmagicdesign.com/support/download/<id>/Linux`; resposta = URL assinada. **Não testado hoje.**
- Catálogo completo: `https://www.blackmagicdesign.com/api/support/us/downloads.json`.

## Repositórios EL9

- **RPM Fusion EL9**: requer EPEL + CRB. `ffmpeg` 7.1.x com libx264, libx265, NVENC, VAAPI, libvpl (QSV); AAC nativo.
- **ROCm 7.2.4**: repo `https://repo.radeon.com/rocm/el9/7.2.4/main`; suporte oficial RHEL 9.4/9.6/9.7 (Rocky só para Instinct). GPUs de consumo oficiais: RX 9070/9060, RX 7900/7800/7700; PRO W7900/W7800/W7700/W6800.
- **Intel NEO** 26.35: upstream só .deb; Gen12+ (Tiger Lake → Battlemage/Lunar/Panther Lake); Gen8–11 em `legacy1` congelado. **Não está no Rocky 9 nem no EPEL**; no EL9 só pelo repo da Intel (`repositories.intel.com/gpu/rhel/...`), voltado a data center.

## Flathub

https://docs.flathub.org/docs/for-app-authors/requirements

- "Thin wrapper or launcher around other tools will not be accepted"; apps que dependem de componentes do host ou setup pós-instalação complicado não são aceitos.
- Nome não pode violar marca nem sugerir afiliação usando o nome do fornecedor/produto (exemplo dado: wrapper de WhatsApp não pode ter "WhatsApp" no nome).
- `--talk-name=org.freedesktop.Flatpak` gera só aviso no linter. Precedentes aprovados: BoxBuddy e DistroShelf (front-ends de distrobox).

## distrobox

- Estável **1.8.2.5**; v2 (Go) em 2.0.0-rc.4.
- Sem root: `curl -fsSL https://raw.githubusercontent.com/89luca89/distrobox/legacy/install | sh -s -- --prefix ~/.local` (garantir `~/.local/bin` no PATH).
- `--nvidia` integra o driver do host (requer host glibc).
- `distrobox-export --app` roda **dentro** do container; `--extra-flags`, `--export-label none`, `--delete`.
