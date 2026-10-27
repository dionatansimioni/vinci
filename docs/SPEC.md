# Resolve Linux Installer — Especificação

> Projeto **não oficial**, sem afiliação com a Blackmagic Design. "DaVinci Resolve" é marca registrada da Blackmagic Design Pty Ltd.

Este documento registra as decisões de design tomadas antes da implementação. Mudanças de direção devem ser registradas aqui.

## 1. Produto

| Item | Decisão |
|---|---|
| Nome | Resolve Linux Installer |
| Público | Comunidade (público, não pessoal) |
| Repositório | `github.com/dionatansimioni/resolve-linux-installer` |
| App ID | `io.github.dionatansimioni.ResolveLinuxInstaller` |
| Licença | GPL-3.0-or-later |
| Telemetria | **Nenhuma.** Botão "Copiar relatório de diagnóstico" para issues |
| Marca | Aviso "não oficial" no README, tela Sobre e Flathub; ícone próprio |
| Idioma | Segue o locale do sistema; pt-BR/inglês selecionável nas configurações; gettext, inglês como texto-fonte e fallback; `.po` no repositório |

## 2. Stack e distribuição

- Rust + gtk4-rs + libadwaita + relm4.
- Lógica em uma crate `core` sem dependência de GTK (testável); UI fina em cima.
- Execução de comandos no host abstraída: direta (AppImage) ou via `flatpak-spawn --host` (Flatpak).
- **Flatpak no Flathub** (principal) + **AppImage** no GitHub Releases com zsync (plano B). Aviso de atualização do app só na AppImage.

## 3. Host

- Qualquer distro. Gerenciadores conhecidos: apt, dnf, pacman, zypper.
- Imutáveis (Silverblue, Kinoite, Bazzite, SteamOS…): detectar; usar o que já existe; se faltar, `rpm-ostree install` + aviso de reboot.
- Distro desconhecida: mostrar o comando manual.
- **podman** via gerenciador de pacotes + `pkexec` (após confirmação).
- **distrobox** instalado em `~/.local` sem root.
- Registro (state file) do que o app instalou. Desinstalação oferece remover só isso; verifica outros containers antes de remover o podman.
- Driver de GPU no host: detectar e orientar; **não** instalar automaticamente.

## 4. Container

- distrobox sobre podman; imagem **Rocky Linux 9** com tag fixada.
- Home compartilhada com o host (padrão do distrobox).
- Uma instalação ativa do Resolve por vez, uma por container.

## 5. GPU

| Fabricante | Estratégia | Limites |
|---|---|---|
| NVIDIA | `distrobox --nvidia`, driver proprietário do host | Sem driver proprietário → orientar |
| AMD | ROCm no container, `/dev/kfd` + `/dev/dri`; `HSA_OVERRIDE_GFX_VERSION` para RDNA1/Vega | Polaris e anteriores: não suportado |
| Intel | `intel-compute-runtime` (NEO) | Arc e Gen12+; anteriores: não suportado |

- Híbridos: dGPU automática, trocável nas configurações; lançador aplica variáveis de PRIME offload.
- Verificação de hardware (GPU, VRAM, RAM, driver, disco): **bloqueia** só o que certamente não funciona (sem GPU compute, disco insuficiente); avisa no resto; opção avançada "instalar mesmo assim". Informa as limitações do hardware do usuário.

## 6. Obtenção do Resolve

- Edições Free e Studio, detectadas pelo nome do arquivo.
- Primeiro: arquivo local (auto-detecção em `~/Downloads`, ou escolhido pelo usuário).
- Fallback: download pela API da Blackmagic com **dados genéricos pré-preenchidos e editáveis**.
- Sempre a última estável, conferida contra uma **lista remota de versões com problema** (JSON no repositório). Se listada: avisar e oferecer a última versão boa.

## 7. Ciclo de vida

- Instalar, atualizar, desinstalar.
- Ao abrir: checar nova versão na API e avisar (sem atualizar sozinho).
- Atualização também atualiza pacotes do container. Botão **Manutenção**: atualizar container, recriar lançador, reaplicar regras udev.
- Projetos/banco de dados: atualização **sempre preserva**; desinstalação mantém por padrão (checkbox desmarcado "apagar também", com confirmação).
- Free → Studio (e vice-versa): nova instalação em container novo; ao final, oferecer remover o container anterior (marcado por padrão).
- Studio: ativação por chave online e por dongle USB (regras udev via `pkexec`).

## 8. Integração com o desktop

- `distrobox-export` do lançador, ícone, associação `.drp`/`.drt`.
- Lançador com variáveis de GPU/PRIME e `QT_QPA_PLATFORM=xcb`.

## 9. Codecs

- ffmpeg do **RPM Fusion dentro do container** (nada redistribuído por nós), com NVENC/VAAPI/QSV quando disponível e fallback CPU.
- Aba **Preparar mídia** (importação): converte para intermediário escolhível — DNxHR HQ (padrão), ProRes 422, ou "só áudio" (AAC→PCM).
- **Pasta monitorada de importação** e **pasta monitorada de exportação**.
- Presets de exportação: "YouTube/Web" (H.264 alta qualidade + AAC 320k), "Arquivo H.265", "Personalizado".
- Serviço em segundo plano (systemd `--user` / portal Background), ícone na bandeja via StatusNotifierItem (testado no GNOME e KDE); sem bandeja → notificações + dica da extensão AppIndicator; desligável nas configurações.

## 10. Testes

- Unitários (Rust) na crate `core` com fixtures: detecção de distro/GPU, parsing da API, nomes de arquivo, comandos gerados.
- CI: integração do container sem GPU em matriz Ubuntu/Fedora/Arch/openSUSE.
- Checklist manual NVIDIA/AMD/Intel com testadores da comunidade antes de cada release.
- Regra do projeto: toda mudança roda a suíte; todo recurso novo testável ganha teste.

## 11. Fatos a verificar

Ver `docs/FACTS.md` (preenchido após pesquisa em fontes primárias).
