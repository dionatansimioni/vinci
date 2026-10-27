# Prova real — Ubuntu 24.04 + NVIDIA (2026-10-27)

Execução do plano de instalação real (`crates/core/examples/run_plan.rs`) nesta máquina de desenvolvimento.

## Ambiente

- Host: Ubuntu 26.04.1 LTS (VM QEMU, 6 threads, 9 GiB RAM), podman 5.7.0 **rootful** (o *linger* do usuário foi bloqueado pela política de permissões; o caminho rootless ainda precisa ser testado).
- distrobox 1.8.2.5 instalado pela receita em prefixo privado (`~/.local/share/vinci/distrobox`).
- GPU: NVIDIA RTX A2000 12GB, driver 595.99.02 (passthrough PCI).
- Instalador: `DaVinci_Resolve_21.1.2_Linux.zip` (Free).

## Resultado

| Passo | Resultado | Tempo |
|---|---|---|
| PullImage (`ubuntu:24.04`) | ok | 4 s |
| Create (`--nvidia`) | ok | < 1 s |
| Initialize | ok | < 1 s |
| InstallPackages | ok | 72 s |
| ExtractInstaller | ok | 52 s |
| InstallResolve | ok | 144 s |
| Verify | ok | < 1 s |
| RemoveConflictingLibs | ok | < 1 s |

- `ldd /opt/resolve/bin/resolve`: nenhuma biblioteca faltando.
- Resolve abriu (Xvfb dentro do container): tela de novidades → Gerenciador de Projetos → criação de projeto.
- Log do Resolve: `Selected compute API: CUDA`, `Let There Be CUDA Light!`, `Nvidia GPU (0) is initialised as decoding and encoding device`, OS detectado `Ubuntu 24.04.5 LTS`.
- ffmpeg 6.1.1 do container: todos os presets de importação/exportação validados com ffprobe (teste `real_ffmpeg_roundtrip`).

## Problemas encontrados e corrigidos

1. **Init do distrobox interrompido** → `sudo` sem `/etc/sudoers`. Novo passo `Initialize`.
2. **Instalador recusa root e sai com código 0 ao abortar** → `installer_args = ["-i","-y","-a"]` na receita e passo `Verify`.
3. **Pacotes faltando** segundo a checagem do próprio instalador: `libice6 libsm6 libxcb-composite0 libxcb-damage0 libxcb-xinput0`.
4. **ffmpeg `-n` sai com código 0** quando recusa sobrescrever → nomes de saída únicos escolhidos antes de rodar.
5. Extração do zip movida para dentro do container (o host pode não ter `unzip`).

## Ainda não testado

- podman **rootless** com usuário comum (caminho real dos usuários).
- AMD (ROCm) e Intel (NEO): sem hardware nesta máquina.
- Edição Studio (ativação/dongle).
- Lançador `.desktop` abrindo o Resolve numa sessão gráfica real.
