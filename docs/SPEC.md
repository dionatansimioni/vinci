# Vinci — Especificação

> Projeto **não oficial**, sem afiliação com a Blackmagic Design. "DaVinci Resolve" é marca registrada da Blackmagic Design Pty Ltd.

Este documento registra as decisões de design tomadas antes da implementação. Mudanças de direção devem ser registradas aqui.

## 1. Produto

| Item | Decisão |
|---|---|
| Nome | **Vinci** — "Instalador não oficial para DaVinci Resolve no Linux" (nome sem a marca, exigência do Flathub) |
| Público | Comunidade (público, não pessoal) |
| Repositório | `github.com/dionatansimioni/vinci` |
| App ID | `io.github.dionatansimioni.Vinci` |
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
- **distrobox**: ver §12.6 (versão fixa em prefixo privado, sem root).
- Registro (state file) do que o app instalou. Desinstalação oferece remover só isso; verifica outros containers antes de remover o podman.
- Driver de GPU no host: detectar e orientar; **não** instalar automaticamente.

## 4. Container

- distrobox sobre podman; imagem **Ubuntu 24.04 LTS** com tag fixada, para todas as GPUs.
  - Motivo (ver FACTS.md): ROCm tem o Ubuntu como distro principal, a Intel publica o NEO em .deb para Ubuntu, e o ffmpeg completo já vem no repositório. Rocky 8.6 é o único listado pela Blackmagic, mas não tem ROCm/NEO atuais.
  - Correções necessárias: `SKIP_PACKAGE_CHECK=1` no instalador e remoção das libs glib/gio embutidas que conflitam com o sistema.
- Home compartilhada com o host (padrão do distrobox).
- Uma instalação ativa do Resolve por vez, uma por container.

## 5. GPU

| Fabricante | Estratégia | Limites |
|---|---|---|
| NVIDIA | `distrobox --nvidia`, driver proprietário do host | Sem driver proprietário → orientar |
| AMD | ROCm no container, `/dev/kfd` + `/dev/dri`; `HSA_OVERRIDE_GFX_VERSION` para RDNA1/Vega | Polaris e anteriores: não suportado |
| Intel | `intel-compute-runtime` (NEO) — **Experimental** (Blackmagic não suporta Intel no Linux) | Arc e Gen12+; anteriores: não suportado |

- Híbridos: dGPU automática, trocável nas configurações; lançador aplica variáveis de PRIME offload.
- Verificação de hardware (GPU, VRAM, RAM, driver, disco): **bloqueia** só o que certamente não funciona (sem GPU compute, disco insuficiente); avisa no resto; opção avançada "instalar mesmo assim". Informa as limitações do hardware do usuário.

## 6. Obtenção do Resolve

- Edições Free e Studio, detectadas pelo nome do arquivo.
- Primeiro: arquivo local (auto-detecção em `~/Downloads`, ou escolhido pelo usuário).
- Fallback: abrir a página oficial de download no navegador e **monitorar `~/Downloads`** até o arquivo chegar, seguindo automaticamente. A API de registro **não** é usada (abandonada pelo AUR; ver FACTS.md). A API pública `latest-stable-version` continua sendo usada para descobrir a versão mais recente.
- Sempre a última estável, conferida contra uma **lista remota de versões com problema** (JSON no repositório). Se listada: avisar e oferecer a última versão boa.

## 7. Ciclo de vida

- Instalar, atualizar, desinstalar.
- Ao abrir: checar nova versão na API e avisar (sem atualizar sozinho).
- Atualização também atualiza pacotes do container. Botão **Manutenção**: atualizar container, recriar lançador, reaplicar regras udev.
- Projetos/banco de dados: atualização **sempre preserva**; desinstalação mantém por padrão (checkbox desmarcado "apagar também", com confirmação).
- Free → Studio (e vice-versa): nova instalação em container novo; ao final, oferecer remover o container anterior (marcado por padrão).
- Studio: ativação por chave online e por dongle USB (regras udev via `pkexec`).
  - As regras vêm do container e passam por lista de permissões: nada que execute programas, e toda mudança de permissão restrita ao vendor Blackmagic (`1edb`). A regra ampla do Resolve (`hidraw` 0777 para tudo) é trocada por uma restrita.

## 8. Integração com o desktop

- `distrobox-export` do lançador, ícone, associação `.drp`/`.drt`.
- Lançador com variáveis de GPU/PRIME e `QT_QPA_PLATFORM=xcb`.

## 9. Codecs

- ffmpeg do **repositório do Ubuntu dentro do container** (nada redistribuído por nós), com NVENC/VAAPI/QSV quando disponível e fallback CPU.
- Aba **Preparar mídia** (importação): converte para intermediário escolhível — DNxHR HQ, ProRes 422, ou "só áudio" (AAC→PCM). Padrão conforme o sistema: Studio + NVIDIA → "só áudio"; demais → DNxHR HQ.
- Motivo: AAC não funciona em nenhuma edição; H.264/H.265 só decodifica no Studio e só codifica no Studio com NVIDIA.
- **Pasta monitorada de importação** e **pasta monitorada de exportação** (padrão `~/Vídeos/Vinci/{Import,Export}`); resultados em subpastas (`Ready for Resolve`, `MP4`) para nunca voltarem como entrada.
- Presets de exportação: "YouTube/Web" (H.264 alta qualidade + AAC 320k), "Arquivo H.265", "Personalizado".
- Serviço em segundo plano (`vinci --watch`, iniciado pela sessão via `~/.config/autostart`, que funciona no GNOME e KDE e com Flatpak/AppImage), ícone na bandeja via StatusNotifierItem (testado no GNOME e KDE); sem bandeja → notificações + dica da extensão AppIndicator; desligável nas configurações.

## 10. Testes

- Unitários (Rust) na crate `core` com fixtures: detecção de distro/GPU, parsing da API, nomes de arquivo, comandos gerados.
- CI: integração do container sem GPU em matriz Ubuntu/Fedora/Arch/openSUSE.
- Checklist manual NVIDIA/AMD/Intel com testadores da comunidade antes de cada release.
- Regra do projeto: toda mudança roda a suíte; todo recurso novo testável ganha teste.

## 11. Fatos a verificar

Ver [FACTS.md](FACTS.md). Pontos que contradizem decisões estão em revisão com o autor.

## 12. Durabilidade ("funcionar para sempre")

Princípio: separar o que muda (dados) do que não muda (código), detectar quebras antes do usuário e sempre ter um caminho de volta.

1. **Receitas declarativas** (TOML versionado): imagem/tag/espelhos, pacotes, libs a remover, variáveis de ambiente, repositórios ROCm/NEO, URLs da Blackmagic, versão fixada do distrobox. Adaptar o app = editar dados.
2. **Receitas remotas** baixadas do GitHub entre releases: só declarativas (sem scripts), **assinadas com minisign** (chave pública embutida), com fallback para a receita embutida.
3. **Migração de base**: quando a receita apontar uma base nova, fluxo igual ao Free→Studio (container novo, validar, oferecer remover o antigo).
4. **Ponto de restauração**: `podman commit` antes de atualizar/migrar; botão "Voltar a esta versão"; guarda só o último; desligável. Medido: ~10 GB e ~7 min para salvar, ~17 s para restaurar.
5. **Cópia do instalador**: guarda o último zip que funcionou em `~/.local/share/vinci/installers`; desligável.
6. **distrobox fixado**: usa o do host se for 1.8.x compatível; senão instala versão fixa em prefixo privado `~/.local/share/vinci/distrobox` (fora do PATH).
7. **Imagem**: `docker.io/library/ubuntu:24.04`, registrando o digest usado; espelhos alternativos na receita.
8. **Autodiagnóstico** a cada abertura (container, lançador, GPU dentro do container, driver do host) com botão "Reparar".
9. **Offline-first**: toda dependência externa é opcional; nada já instalado deixa de funcionar se GitHub ou Blackmagic mudarem.
10. **Vigia semanal no CI**: monta o container pela receita, confere pacotes, API da Blackmagic e repositórios; abre issue se quebrar.
