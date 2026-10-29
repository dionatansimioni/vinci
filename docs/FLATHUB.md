# Viabilidade da publicação no Flathub

Análise de 2026-10-29, versão 0.4.4.

## Conclusão

**Viável, mas não de imediato.** Nada no Vinci viola uma regra do Flathub de forma
incontornável: há apps aprovados com o mesmo modelo (BoxBuddy e DistroShelf usam
`flatpak-spawn --host` para gerenciar containers). Ainda assim, faltam trabalho técnico e
testes, e a aprovação depende da avaliação dos revisores sobre as permissões.

Recomendação:
1. Publicar primeiro no GitHub com o AppImage.
2. Juntar testes da comunidade em AMD, Intel e GNOME.
3. Depois disso, submeter ao Flathub com os itens abaixo resolvidos.

## Pontos a favor

- **Nome sem a marca.** "Vinci" não usa "DaVinci Resolve" no nome, e o aviso de projeto
  não oficial já está no metainfo, na tela Sobre, no README e no site.
- **Nada proprietário é redistribuído.** O usuário baixa o Resolve da Blackmagic. Isso tem o
  mesmo espírito de apps aceitos que instalam software de terceiros, como Bottles e
  ProtonUp-Qt.
- **Licença livre (GPL-3.0) e build a partir do código-fonte.**
- **App ID `io.github.dionatansimioni.Vinci`.** Ele corresponde ao repositório
  `github.com/dionatansimioni/vinci`, e o selo de verificado sai pelo login com a conta do
  GitHub.
- **Metainfo completo.** Ele valida com `appstreamcli` e agora tem screenshots, histórico de
  versões, OARS e links.
- **Runtime GNOME 49 e manifesto já escritos** (`build-aux/flatpak/`).

## Riscos na revisão

| Permissão | Por que o Vinci precisa | Risco |
|---|---|---|
| `--talk-name=org.freedesktop.Flatpak` | Rodar podman, distrobox e pkexec no host. Sem isso o app não funciona. | **Alto.** É uma saída total do sandbox; os revisores pedem justificativa e podem recusar. Os precedentes BoxBuddy e DistroShelf ajudam. |
| `--filesystem=home` | Instalador em `~/Downloads`, mídia em qualquer pasta, pastas monitoradas | Médio. Dá para justificar, mas é provável que peçam algo mais restrito (`xdg-download`, `xdg-videos` + portal de arquivos). |
| `--share=network` | Versão mais recente e lista de versões com problemas | Baixo |
| `--talk-name=org.kde.StatusNotifierWatcher` | Ícone na bandeja | Baixo (é comum) |

## O que falta fazer

1. **Testar o build Flatpak de verdade.** `flatpak-builder` nunca rodou: falta gerar o
   `cargo-sources.json` (`flatpak-cargo-generator.py`) e fazer uma instalação completa do
   Resolve de dentro do sandbox.
2. **Manifesto para o Flathub.** Trocar `type: dir` por `type: git` com a tag e o commit
   do release. O Flathub compila a partir de um release publicado.
3. **Autostart pelo portal.** Dentro do Flatpak, o serviço de pastas monitoradas deve pedir
   para iniciar com a sessão pelo portal `org.freedesktop.portal.Background`, em vez de
   gravar em `~/.config/autostart`. Os revisores costumam exigir isso.
4. **Reduzir `--filesystem=home`, se for pedido.** Usar o portal de arquivos para escolher
   instaladores e mídia, e manter só `xdg-download` e `xdg-videos`.
5. **Escrever a justificativa das permissões** no pull request de submissão, citando BoxBuddy
   e DistroShelf.
6. **Branding no metainfo.** Adicionar `<branding>` com as cores do ícone (`#e66100` e
   `#3d3846`) para a página do Flathub.
7. **Atualização do app.** Dentro do Flatpak, quem atualiza é o Flathub. O aviso de
   atualização do AppImage já fica desligado lá; confirmar isso no teste.

## Esforço estimado

- Itens 1, 2, 5 e 6: um ou dois dias, a maior parte testando.
- Item 3: cerca de um dia, com teste.
- Item 4: só se os revisores pedirem.
- A revisão do Flathub costuma levar de alguns dias a algumas semanas, com uma ou mais
  rodadas de ajustes.
