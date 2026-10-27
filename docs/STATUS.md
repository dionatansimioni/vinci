# Status (2026-10-27)

## Pronto e testado

| Área | Situação |
|---|---|
| Verificação de hardware (GPU, driver, RAM, disco, híbridos) | ✅ unitário + tela real |
| Receita declarativa assinada (minisign) + cache offline | ✅ unitário (chave de produção pendente) |
| Preparação do host (podman via pkexec, distrobox fixado em prefixo privado) | ✅ unitário; ✅ distrobox real |
| Plano de instalação + executor com recuperação (espelhos, init) | ✅ unitário; ✅ real (NVIDIA) |
| Fluxo completo: atalho, ícone, MIME, estado, cópia do instalador | ✅ real; atalho abre o Resolve com CUDA |
| Autodiagnóstico + Reparar | ✅ real (atalho apagado → recriado) |
| Desinstalação seletiva | ✅ unitário |
| Conversão de mídia (import/export, progresso, nomes únicos) | ✅ real dentro do container |
| Aviso de nova versão / versões com problema | ✅ unitário + API real |
| Relatório de diagnóstico (sem telemetria, anonimizado) | ✅ unitário |
| Interface: verificação, instalador, progresso, conclusão/erro, Mídia, desinstalar, troca de edição | ✅ compilada e conferida em Xvfb |
| pt-BR completo, com teste que exige tradução e preserva marcadores | ✅ |
| Vigia semanal (watchdog) + CI | ✅ watchdog executado localmente |
| Empacotamento Flatpak (manifesto, AppStream, ícone) | ✅ arquivos validados; build não executado |
| Script AppImage | ⚠️ escrito, **não executado** |

## Pendente

1. **Pastas monitoradas em segundo plano + ícone na bandeja** (SPEC §9): núcleo pronto (`convert::pending`, `convert_file`); falta o serviço (systemd `--user`/portal Background) e o StatusNotifierItem.
2. **Ponto de restauração** antes de atualizar (`podman commit`) e botão "Voltar à versão anterior" (SPEC §12.4).
3. **Configurações**: idioma (pt-BR/inglês), GPU preferida em híbridos, guardar instalador, pastas monitoradas.
4. **Studio com dongle USB**: regras udev via pkexec (SPEC §7).
5. **Chave de assinatura das receitas** (minisign): gerar e guardar fora do repositório; colocar a pública em `recipe::PUBLIC_KEY`.
6. **Testes que dependem de você/ambiente**: podman rootless com usuário comum; GNOME/KDE reais (Wayland+XWayland); AMD e Intel; Studio; build Flatpak e AppImage.
7. **Publicar no GitHub** (`github.com/dionatansimioni/vinci`) — só com sua autorização.
