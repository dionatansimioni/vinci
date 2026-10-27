# Empacotamento

## Flatpak (canal principal)

```sh
# Gera as fontes do Cargo para build offline (exigência do Flathub)
python3 flatpak-cargo-generator.py Cargo.lock -o build-aux/flatpak/cargo-sources.json
flatpak-builder --user --install --force-clean build-flatpak build-aux/flatpak/io.github.dionatansimioni.Vinci.yml
flatpak run io.github.dionatansimioni.Vinci
```

O `flatpak-cargo-generator.py` vem de <https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo>.

Permissões e motivos estão comentados no manifesto. Dentro do sandbox, o Vinci executa
podman/distrobox/pkexec no host com `flatpak-spawn --host` e grava o atalho do Resolve nos
diretórios XDG **do host** (`HOST_XDG_DATA_HOME`).

## AppImage (plano B)

Veja `build-aux/appimage/build.sh`.
