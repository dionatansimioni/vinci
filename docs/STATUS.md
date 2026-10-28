# Status (2026-10-28, versão 0.2.0)

## Pronto e testado

| Área | Situação |
|---|---|
| Verificação de hardware (GPU, driver, RAM, disco, híbridos) | ✅ unitário + tela real |
| Receita declarativa embutida, validada com rigor | ✅ unitário |
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
| AppImage (38 MB, GLIBC ≥ 2.39) | ✅ gerada e testada num Ubuntu 24.04 limpo sem GTK |
| Ponto de restauração + "Voltar a esta versão" | ✅ real (10 GB / 7 min para salvar, 17 s para restaurar) |
| Preferências (idioma, GPU, restauração, instalador, pastas) | ✅ unitário + tela real |
| Serviço de pastas monitoradas (`vinci --watch`) + autostart | ✅ real (conversão, sem reconversão, desligamento, instância única) |
| Regras udev (dongle, Speed Editor, painéis) com validação de segurança | ✅ unitário + regras reais validadas |
| Barra de progresso também na instalação do podman/distrobox | ✅ unitário; ✅ real (distrobox reinstalado pela interface) |
| Menu principal com Preferências e "Sobre o Vinci" | ✅ tela real |
| Ícone na barra de tarefas (ícone da janela + atalho próprio da AppImage) | ✅ unitário; ⏳ conferir no KDE/GNOME reais |
| Botões minimizar/maximizar/fechar em qualquer desktop | ✅ unitário + Xvfb; ⏳ conferir no KDE real |
| Conclusão → "Abrir" volta à visão da instalação sem reiniciar | ✅ real |
| Mídia: "Mostrar na pasta" e aviso de tamanho por perfil | ✅ unitário + conversão real |
| Preferências > Avançado: terminal no container e pasta da instalação | ✅ unitário; terminal conferido com shell real no container |
| AppImage com a versão no nome (`Vinci-<versão>-x86_64.AppImage`) | ✅ teste de consistência versão/metainfo |

## Pendente

1. **Testes que dependem de você/ambiente**: podman rootless com usuário comum; GNOME/KDE reais (bandeja, notificações, Wayland+XWayland, atalho no menu); AMD e Intel; Studio com chave e com dongle; build Flatpak.
2. **Publicar no GitHub** — só quando você disser que está pronto.
