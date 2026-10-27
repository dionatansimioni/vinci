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

## Segunda rodada — fluxo completo (`flow::install_resolve`)

Mesma máquina, executando a função que a interface usa (`crates/core/examples/install.rs`): plano + atalho + ícone + MIME + estado + cópia do instalador.

| Passo | Tempo |
|---|---|
| PullImage | 2 s |
| RemoveExisting (reinstalação) | 3 s |
| Create | < 1 s |
| Initialize (init do distrobox, primeira vez) | 197 s |
| InstallPackages | 65 s |
| ExtractInstaller | 57 s |
| InstallResolve | 167 s |
| Verify + RemoveConflictingLibs | < 1 s |

Resultado: `installed vinci-free 21.1.2`. Atalho válido (`desktop-file-validate`), ícone 128×128 instalado, `*.drp` registrado no banco MIME, instalador guardado por hard link (sem espaço extra) e pasta de trabalho apagada.

Bugs que só a execução real revelou (todos corrigidos, com teste):

6. `unzip -d` não cria pastas intermediárias (`~/.cache/vinci` não existe numa máquina nova).
7. Reinstalar falharia: `distrobox create` recusa nome existente → passo `RemoveExisting` depois do download.
8. `Exec=` do atalho saiu com `distrobox` sem caminho absoluto → numa sessão gráfica com outro PATH o atalho não abriria.
9. `apt-get update` sai com 0 mesmo sem rede → `--error-on=any` (achado pelo vigia).

Conversão de mídia testada dentro do container: todos os presets, progresso ao vivo e API da Blackmagic pela rede do container.

## Ainda não testado

- podman **rootless** com usuário comum (caminho real dos usuários).
- AMD (ROCm) e Intel (NEO): sem hardware nesta máquina.
- Edição Studio (ativação/dongle).
- Lançador `.desktop` abrindo o Resolve numa sessão gráfica real.
