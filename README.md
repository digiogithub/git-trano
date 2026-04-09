# git-trano

Plugin de Git escrito en Rust para despliegues estilo **Capistrano** desde el repositorio actual.

Permite:

- Sincronizar remoto (`git fetch`)
- Desplegar una **rama** o una **tag**
- Crear releases en `releases/<fecha-hora>`
- Actualizar el symlink `current`
- Mantener sólo las últimas _N_ releases
- Hacer `revert` a la release anterior
- Gestionar symlinks de rutas compartidas (`--shared`)

---

## Características

- Comando estilo plugin: `git trano ...`
- Estructura de despliegue:
  - `<path>/releases`
  - `<path>/current` (symlink)
  - `<path>/shared`
- Despliegue atómico por cambio de symlink
- Limpieza automática de versiones antiguas (`--keep`, por defecto `3`)
- Soporte para múltiples `--shared`
- Makefile con tareas de build, static build e instalación
- Pipeline de GitHub Actions para generar binarios estáticos (musl) y publicarlos en releases
- Compatible con Linux/macOS (symlinks POSIX)

---

## Estructura generada en destino

Dado `--path /www/folder`, se genera:

```/dev/null/tree.txt#L1-8
/www/folder/
├── current -> /www/folder/releases/2026-01-01T12-00-00Z
├── releases/
│   ├── 2026-01-01T11-00-00Z/
│   └── 2026-01-01T12-00-00Z/
└── shared/
    ├── node_modules/
    └── vendor/subfolder/
```

---

## Requisitos

- Git instalado y disponible en `PATH`
- Rust estable (si compilas desde código)
- Ejecutar dentro de un repositorio Git válido

---

## Instalación

### Opción 1: Compilar desde código (cargo)

```/dev/null/install-build.sh#L1-4
cargo build --release
install -m 0755 target/release/git-trano /usr/local/bin/git-trano
# opcional alias plugin:
ln -sf /usr/local/bin/git-trano /usr/local/bin/git-trano
```

Luego podrás usar:

```/dev/null/usage.txt#L1-1
git trano ...
```

> Git invoca subcomandos mediante binarios con prefijo `git-<subcomando>`.  
> Para `git trano`, el ejecutable debe llamarse `git-trano`.

### Opción 2: `cargo install` local

```/dev/null/cargo-install.sh#L1-1
cargo install --path .
```

Asegúrate de que `~/.cargo/bin` esté en tu `PATH`.

### Opción 3: Usar Makefile

```/dev/null/make-quickstart.sh#L1-6
make help
make build
make release
make static
make static-all
make install
```

---

## Uso

```/dev/null/help.txt#L1-14
git trano --branch <nombre_rama> --path <ruta_destino> [--keep <n>] [--shared <ruta>]...
git trano --tag <tag> --path <ruta_destino> [--keep <n>] [--shared <ruta>]...
git trano --revert --path <ruta_destino>

Opciones:
  -b, --branch <rama>     Despliega la rama indicada
  -t, --tag <tag>         Despliega la tag indicada
  -p, --path <ruta>       Ruta base de despliegue
  -k, --keep <n>          Releases a mantener (default: 3)
      --shared <ruta>     Ruta compartida (repetible)
  -r, --revert            Apunta current a la release anterior
  -h, --help              Ayuda
  -V, --version           Versión
```

---

## Ejemplos

### Desplegar rama

```/dev/null/examples.txt#L1-2
git trano --branch main --path /www/folder --keep 5
git trano -b main -p /www/folder
```

Qué hace:

1. `git fetch --all --prune`
2. Actualiza checkout local a la rama indicada
3. Copia el directorio de trabajo actual a:
   - `/www/folder/releases/<fecha-hora>`
4. Reemplaza symlink:
   - `/www/folder/current -> /www/folder/releases/<fecha-hora>`
5. Elimina releases antiguas y conserva las últimas `N`

### Desplegar tag

```/dev/null/examples.txt#L3-4
git trano --tag v1.2.3 --path /www/folder
git trano -t v1.2.3 -p /www/folder
```

Mismo flujo que rama, pero usando checkout de tag.

### Revert a release anterior

```/dev/null/examples.txt#L6-6
git trano --revert --path /www/folder
```

- Toma las releases ordenadas por fecha
- Apunta `current` a la penúltima release disponible

### Shared links

```/dev/null/examples.txt#L8-8
git trano --branch main --path /www/folder --shared node_modules --shared vendor/subfolder
```

Después de actualizar `current`:

- asegura `/www/folder/shared/node_modules`
- asegura `/www/folder/shared/vendor/subfolder`
- si existen en `current`, los elimina
- crea symlink:
  - `/www/folder/current/node_modules -> /www/folder/shared/node_modules`
  - `/www/folder/current/vendor/subfolder -> /www/folder/shared/vendor/subfolder`

---

## Flujo interno (resumen)

1. Validación de argumentos (rama/tag/revert mutuamente excluyentes)
2. Preparación de directorios base (`releases`, `shared`)
3. Modo deploy:
   - fetch remoto
   - checkout rama/tag
   - crear release timestamp
   - copiar archivos del repo a release (excluyendo `.git`)
   - actualizar symlink `current`
   - aplicar `shared`
   - limpieza de releases antiguas
4. Modo revert:
   - listar releases
   - mover `current` a release anterior

---

## Compilación estática

> Nota: en Linux glibc, los binarios suelen ser dinámicos por defecto.  
> Para binario estático real, usa **musl**.

### Linux x86_64 estático (musl)

```/dev/null/static-build.sh#L1-3
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
strip target/x86_64-unknown-linux-musl/release/git-trano
```

Binario resultante:

```/dev/null/static-build.sh#L5-5
target/x86_64-unknown-linux-musl/release/git-trano
```

Verificar que sea estático:

```/dev/null/static-check.sh#L1-1
ldd target/x86_64-unknown-linux-musl/release/git-trano
```

Debería indicar que no es ejecutable dinámico (o equivalente).

### Cross-compilation (opcional con `cross`)

```/dev/null/cross.sh#L1-2
cargo install cross
cross build --release --target x86_64-unknown-linux-musl
```

## Makefile incluido

El proyecto incorpora un `Makefile` con objetivos principales:

```/dev/null/make-targets.txt#L1-13
make help
make check
make fmt
make clippy
make test
make build
make release
make static
make static-x86_64
make static-aarch64
make static-all
make install
make uninstall
```

Notas:
- `make static` compila `x86_64-unknown-linux-musl`.
- `make static-all` compila `x86_64` y `aarch64` en musl.
- `make install` instala por defecto en `/usr/local/bin/git-trano`.
- Puedes personalizar instalación con `PREFIX`, `BINDIR` y `DESTDIR`.

Ejemplo:

```/dev/null/make-install-example.sh#L1-1
make install PREFIX=/usr DESTDIR=/tmp/pkgroot
```

## Pipeline de GitHub Actions para binarios estáticos

Se incluye el workflow:

- `.github/workflows/release-static.yml`

Qué hace:
1. Se ejecuta en tags `v*` (y también manual con `workflow_dispatch`).
2. Compila binarios estáticos `musl` para:
   - `x86_64-unknown-linux-musl`
   - `aarch64-unknown-linux-musl`
3. Empaqueta cada binario en `.tar.gz`.
4. Genera checksum `.sha256` por artefacto.
5. Publica artefactos del workflow.
6. Si el disparador es un tag, sube assets automáticamente al GitHub Release.
7. Genera también un `SHA256SUMS` combinado y lo adjunta al release.

### Cómo publicar una release estática

```/dev/null/release-flow.sh#L1-7
git tag v0.1.0
git push origin v0.1.0
# GitHub Actions compilará y adjuntará:
# - git-trano-v0.1.0-linux-amd64-musl.tar.gz
# - git-trano-v0.1.0-linux-amd64-musl.tar.gz.sha256
# - git-trano-v0.1.0-linux-arm64-musl.tar.gz
# - git-trano-v0.1.0-linux-arm64-musl.tar.gz.sha256
```

---

## Consideraciones y buenas prácticas

- Ejecuta `git trano` desde el repo correcto (working tree limpio recomendado)
- Para producción, usa un usuario con permisos limitados sobre `--path`
- Verifica espacio en disco si aumentas `--keep`
- Usa `--shared` para datos persistentes entre releases (`uploads`, `storage`, `node_modules`, etc.)
- Si despliegas tags, idealmente usa tags inmutables firmadas

---

## Solución de problemas

### `git trano` no se encuentra

Verifica que `git-trano` esté en el `PATH`:

```/dev/null/troubleshoot.sh#L1-2
which git-trano
git trano --help
```

### Error de permisos en destino

Ajusta propietario/permisos sobre `<path>`:

```/dev/null/troubleshoot.sh#L4-6
sudo mkdir -p /www/folder
sudo chown -R <usuario>:<grupo> /www/folder
chmod -R u+rwX /www/folder
```

### No existe rama/tag remota

Confirma disponibilidad:

```/dev/null/troubleshoot.sh#L8-9
git fetch --all --prune
git branch -a && git tag
```

---

## Roadmap sugerido

- Hook pre/post deploy
- Modo `--dry-run`
- Filtro de exclusiones configurable (`--exclude`)
- Locks para despliegues concurrentes
- Integración con notificaciones (Slack/Discord/Webhook)

---

## Licencia

MIT. Ver archivo `LICENSE`.