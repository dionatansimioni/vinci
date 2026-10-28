# Status (2026-10-28)

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
| Ponto de restauração + "Voltar a esta versão" | ✅ real (10 GB / 7 min para salvar, 17 s para restaurar) |
| Preferências (idioma, GPU, restauração, instalador, pastas) | ✅ unitário + tela real |
| Serviço de pastas monitoradas (`vinci --watch`) + autostart | ✅ real (conversão, sem reconversão, desligamento, instância única) |
| Regras udev (dongle, Speed Editor, painéis) com validação de segurança | ✅ unitário + regras reais validadas |

## Pendente

1. **Chave de assinatura**: gerada; falta você colocar senha e guardar cópias (veja README de assinatura em `scripts/sign-recipe.sh`).
2. **Testes que dependem de você/ambiente**: podman rootless com usuário comum; GNOME/KDE reais (bandeja, notificações, Wayland+XWayland, atalho no menu); AMD e Intel; Studio com chave e com dongle; build Flatpak e AppImage.
3. **Publicar no GitHub** — só quando você disser que está pronto.
