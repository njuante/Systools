# 1. Visión del producto

## Nombre funcional

**SysTUI**

## Propuesta real

Suite TUI en Rust para administración de servidores Linux, enfocada en:

Local mode:

```bash
systui
```

Remote mode:

```bash
systui ssh user@server
```

Fleet mode:

```bash
systui fleet
```

Read-only mode:

```bash
systui --read-only
```

Report mode:

```bash
systui report --host prod-01 --format html
```

## Principios del proyecto

La aplicación debe cumplir estos principios desde el diseño inicial:

1. **Agentless first**: no instalar nada en el servidor remoto. Todo vía SSH o ejecución local.
2. **Seguro por defecto**: modo solo lectura, confirmaciones para acciones destructivas, preview antes de modificar.
3. **Auditable**: cada acción queda registrada con usuario, host, comando lógico, resultado y timestamp.
4. **Cross-distro**: Debian, Ubuntu, Arch, Fedora, Rocky/Alma, openSUSE y Alpine como objetivos progresivos.
5. **No depender de un daemon propio**: SysTUI debe funcionar como binario único.
6. **Lectura objetiva del sistema**: no solo mostrar datos, sino detectar problemas.
7. **TUI rápida y agradable**: navegación fluida, buscador, filtros, atajos, paneles claros.
8. **Extensible**: arquitectura modular para añadir módulos sin romper el núcleo.

---

# 2. Arquitectura general recomendada

No lo hagas como una app monolítica. Hazlo como workspace Rust con crates separados.

Estructura recomendada:

```text
systui/
├── crates/
│   ├── systui-cli/              # Binario principal
│   ├── systui-core/             # Modelos, errores, configuración, contratos
│   ├── systui-ui/               # Ratatui, componentes, navegación
│   ├── systui-transport/        # Local, SSH, futuro agente opcional
│   ├── systui-collectors/       # Lectura de sistema, logs, red, docker...
│   ├── systui-actions/          # Acciones: restart service, kill process, edit cron...
│   ├── systui-security/         # Checks de seguridad y hardening
│   ├── systui-report/           # Exportación Markdown/HTML/JSON/PDF futuro
│   ├── systui-storage/          # Config, perfiles, caché, auditoría local
│   └── systui-testkit/          # Fixtures, mocks, contenedores de prueba
├── docs/
├── examples/
├── packaging/
└── README.md
```

## Capas internas

La app debería tener cinco capas:

```text
UI TUI
  ↓
Application State / Event Bus
  ↓
Collectors + Actions
  ↓
Transport Layer
  ↓
Local system / SSH remote host
```

## Contrato fundamental

Todo módulo debe funcionar igual en local o remoto. Para eso necesitas una abstracción tipo:

```rust
trait Transport {
    async fn run(&self, command: CommandSpec) -> Result<CommandOutput>;
    async fn read_file(&self, path: &str) -> Result<Vec<u8>>;
    async fn file_exists(&self, path: &str) -> Result<bool>;
    async fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>>;
}
```

Luego implementas:

```text
LocalTransport
SshTransport
MockTransport
```

Esto es clave. Si no separas bien transporte y módulos, luego el SSH será un infierno.

---

# 3. Modelo de seguridad de la aplicación

Esta parte debe existir desde la fase 1, no al final.

## Modos de ejecución

SysTUI debería tener tres modos principales:

```text
Read-only
Safe actions
Privileged actions
```

### Read-only

No modifica nada. Solo inspecciona.

Ejemplos:

```bash
systui --read-only
systui ssh root@server --read-only
```

### Safe actions

Permite acciones reversibles o de bajo riesgo:

```text
ver logs
filtrar procesos
hacer ping
exportar reporte
listar crons
comprobar puertos
```

### Privileged actions

Requiere confirmación explícita:

```text
restart service
stop service
kill process
borrar cron
editar crontab
limpiar logs
borrar contenedor
reiniciar servidor
modificar firewall
```

## Confirmaciones inteligentes

No uses una confirmación genérica tipo:

```text
Are you sure? y/N
```

Para acciones peligrosas usa confirmación contextual:

```text
Vas a reiniciar nginx en prod-01.
Esto puede cortar tráfico web activo.

Escribe: restart nginx
> 
```

## Dry-run obligatorio

Siempre que se pueda:

```text
Preview
Diff
Dry-run
Apply
Rollback info
```

Por ejemplo, al editar un cron:

```text
Antes:
0 2 * * * /backup.sh

Después:
0 3 * * * /backup.sh

Backup creado:
~/.config/systui/backups/prod-01/crontab-2026-05-23-1430.bak
```

## Auditoría local

Todo lo que modifique algo debe quedar en un log local:

```json
{
  "timestamp": "2026-05-23T14:31:22Z",
  "host": "prod-01",
  "user": "admin",
  "module": "services",
  "action": "restart",
  "target": "nginx.service",
  "status": "success",
  "duration_ms": 841
}
```

---

# 4. Funcionalidades completas por módulos

## 4.1 Dashboard principal

Este debe ser el centro de la aplicación.

No debe limitarse a mostrar CPU y RAM. Debe responder a:

```text
¿Está sano este servidor?
¿Qué está fallando?
¿Qué consume más?
¿Qué está expuesto?
¿Qué ha cambiado?
¿Qué debería revisar primero?
```

Paneles recomendados:

```text
Health score
CPU / RAM / Swap
Disk usage
Failed systemd units
Top processes
Open ports
Network traffic
Docker status
Recent critical logs
Security findings
Pending updates
Certificate expiration
Backup status
Uptime
Load average
Logged users
```

Ejemplo de dashboard:

```text
┌─ prod-01 ──────────────────────────────── Health: 78/100 ─┐
│ CPU 18%   RAM 62%   Disk / 84%   Load 0.82                 │
│ Failed units: 2   Open ports: 8   Critical logs: 14        │
│ Docker: 12 running / 1 unhealthy   Updates: 23 pending     │
└────────────────────────────────────────────────────────────┘

Critical findings:
[HIGH] / partition above 80%
[HIGH] sshd allows password authentication
[MED] nginx has 7 recent 5xx errors
[MED] certificate for api.example.com expires in 12 days
[LOW] 3 zombie processes detected
```

Funcionalidad clave: **el dashboard debe priorizar problemas**, no vomitar métricas.

---

## 4.2 Sistema y hardware

Funcionalidades necesarias:

```text
CPU actual
CPU histórica durante la sesión
RAM
Swap
Load average
Uptime
Kernel
Distribución
Hostname
Arquitectura
Usuarios conectados
Temperaturas si existen sensores disponibles
Discos por partición
Uso de inodos
Mountpoints
I/O básica
SMART opcional si smartctl existe
RAID/mdadm si existe
```

Checks útiles:

```text
Partición por encima de 80%, 90%, 95%
Inodos por encima de 80%
Swap usada de forma anormal
Load average alto frente al número de cores
Uptime excesivo con kernel antiguo
OOM kills recientes
Dmesg con errores críticos
Filesystem en modo read-only
```

Acciones:

```text
Ver procesos que consumen disco
Ver directorios más pesados
Ver logs relacionados con OOM
Exportar snapshot
```

Evita implementar un “limpiador mágico” agresivo. Es mejor detectar y guiar.

---

## 4.3 Procesos

Funcionalidades:

```text
Lista de procesos
Orden por CPU
Orden por RAM
Filtro por usuario
Filtro por nombre
Árbol de procesos
Ver comando completo
Ver cwd si hay permisos
Ver ficheros abiertos si lsof existe
Ver puertos asociados
Matar proceso
Enviar señales: SIGTERM, SIGKILL, SIGHUP
```

Acciones seguras:

```text
Primero SIGTERM
Luego opción de SIGKILL
Confirmación si el proceso es crítico
Bloqueo por defecto para PID 1, sshd, systemd, proceso actual de SysTUI
```

Detecciones útiles:

```text
Procesos zombie
Procesos con consumo alto sostenido
Procesos ejecutados desde /tmp
Procesos con binarios borrados
Procesos escuchando en puertos externos
Procesos sospechosos por usuario inesperado
```

---

## 4.4 Servicios systemd

Este módulo debe ser prioritario.

Funcionalidades:

```text
Listar servicios
Filtrar activos, fallidos, enabled, disabled
Ver estado completo
Ver logs del servicio
Start
Stop
Restart
Reload
Enable
Disable
Mask
Unmask
Ver unit file
Ver override
Ver dependencias
Ver tiempo de arranque
```

Checks útiles:

```text
Unidades fallidas
Servicios enabled pero muertos
Servicios reiniciando en bucle
Servicios con logs críticos recientes
Servicios con ExecStart apuntando a rutas inexistentes
Servicios con permisos inseguros en unit files
```

Acciones:

```text
restart service
reload service
enable/disable service
abrir logs filtrados de ese servicio
crear reporte del servicio
```

UX importante:

Desde un servicio deberías poder pulsar una tecla y saltar a:

```text
logs del servicio
proceso asociado
puertos asociados
ficheros de configuración comunes
```

Ejemplo:

```text
nginx.service
 ├─ status: active
 ├─ enabled: true
 ├─ pid: 1132
 ├─ ports: 80, 443
 ├─ recent errors: 7
 └─ actions: logs | restart | reload | config | report
```

---

## 4.5 Logs

Este módulo debe ser uno de los más potentes.

Fuentes:

```text
journald
/var/log/syslog
/var/log/messages
/var/log/auth.log
/var/log/secure
nginx logs
apache logs
docker logs
custom files
JSON logs
```

Funcionalidades:

```text
Tail en tiempo real
Filtro por nivel
Filtro por fecha
Filtro por servicio
Filtro por regex
Filtro por host
Búsqueda incremental
Agrupación de errores repetidos
Detección de picos
Exportar fragmento
Marcar líneas importantes
```

Funciones diferenciales:

```text
Resumen automático de errores recientes
Agrupación por fingerprint
Contador de eventos por minuto
Detección de mensajes nuevos desde que abriste la app
Filtro “solo errores nuevos”
```

Ejemplo:

```text
Recent log summary:
[42x] nginx: upstream timed out
[17x] sshd: failed password for invalid user
[9x] postgres: deadlock detected
[3x] kernel: blocked task for more than 120 seconds
```

Acciones:

```text
Ir al servicio relacionado
Ir al proceso relacionado
Crear reporte
Guardar búsqueda como preset
```

---

## 4.6 Red y conectividad

Funcionalidades mínimas:

```text
Interfaces activas
IPs
Rutas
DNS configurado
Puertos abiertos
Puertos por proceso
Conexiones activas
Conexiones por estado
Tráfico por interfaz
Ping
Traceroute si existe
Test DNS
Test TCP connect
```

Checks útiles:

```text
Puerto escuchando en 0.0.0.0
Puerto sensible expuesto: 22, 3306, 5432, 6379, 9200, 27017
DNS roto
Gateway no accesible
Muchas conexiones TIME_WAIT
Muchas conexiones SYN_RECV
Servicio escuchando sin proceso identificado
```

Acciones:

```text
Ver proceso de un puerto
Ver servicio systemd asociado
Probar conexión a host:puerto
Resolver dominio
Exportar mapa de exposición
```

Diferenciador clave:

```text
Exposure Map
```

Ejemplo:

```text
External exposure:
0.0.0.0:22     sshd       HIGH
0.0.0.0:443    nginx      OK
127.0.0.1:5432 postgres   OK
0.0.0.0:6379   redis      CRITICAL
```

Este módulo da muchísimo valor real.

---

## 4.7 Docker y contenedores

Funcionalidades:

```text
Listar contenedores running/stopped
Estado de salud
Imagen
Puertos publicados
Volúmenes
Redes
Uso de CPU/RAM
Logs en tiempo real
Start
Stop
Restart
Remove con confirmación
Inspect resumido
Stats
Ver compose project si aplica
```

Checks útiles:

```text
Contenedores unhealthy
Contenedores reiniciando en bucle
Puertos sensibles publicados
Contenedores corriendo como privileged
Montaje de docker.sock
Montajes peligrosos: /, /etc, /var/run
Imagen con tag latest
Contenedores sin límite de memoria
Logs excesivamente grandes
```

Acciones:

```text
Ver logs
Restart
Shell opcional
Ver puertos
Ver volúmenes
Ver riesgos
Exportar compose summary
```

Futuro:

```text
Podman support
Compose project view
Kubernetes local/minikube/k3s opcional
```

Pero no metas Kubernetes en la primera versión. Puede desviar demasiado el proyecto.

---

## 4.8 Crons y timers

Este módulo puede diferenciar mucho a SysTUI porque los crons son una fuente constante de errores.

Fuentes:

```text
crontab por usuario
/etc/crontab
/etc/cron.d/*
/etc/cron.daily
/etc/cron.hourly
/etc/cron.weekly
systemd timers
```

Funcionalidades:

```text
Listar crons por usuario
Crear cron
Editar cron
Eliminar cron
Desactivar temporalmente
Validar expresión
Preview en lenguaje natural
Próximas ejecuciones
Historial estimado
Logs relacionados
Detección de scripts inexistentes
Detección de permisos incorrectos
```

Checks útiles:

```text
Cron ejecuta script que no existe
Cron ejecuta script sin permisos
Cron redirige mal logs
Cron corre como root y escribe en rutas inseguras
Cron duplicado
Cron con frecuencia sospechosamente alta
Cron sin logging
```

Ejemplo:

```text
0 2 * * * /opt/backup.sh

Runs:
Today 02:00
Tomorrow 02:00
Every day at 02:00

Warnings:
[MED] No stdout/stderr redirection detected
[HIGH] /opt/backup.sh does not exist
```

Acción diferencial:

```text
Convertir cron a systemd timer
```

No lo metas en MVP, pero sí en roadmap avanzado.

---

## 4.9 Bases de datos

Este módulo tiene que ser cuidadoso. No debe intentar ser un cliente SQL completo. Debe ser un panel operativo.

Soporte progresivo:

```text
PostgreSQL
MySQL/MariaDB
Redis
MongoDB
```

Funcionalidades por base de datos:

```text
Estado del servicio
Puerto
Versión
Conexiones activas
Queries lentas si está disponible
Tamaño de bases/tablas
Locks
Replicación
Backups detectados
Errores recientes en logs
```

PostgreSQL:

```text
pg_stat_activity
pg_locks
pg_stat_database
replication status
database sizes
slow queries si pg_stat_statements está disponible
```

MySQL/MariaDB:

```text
processlist
slow query log
table sizes
replication status
connection usage
```

Redis:

```text
INFO
memory usage
connected clients
persistence status
replication
evicted keys
blocked clients
```

MongoDB:

```text
serverStatus
connections
replication
database sizes
slow operations si está disponible
```

Checks útiles:

```text
Demasiadas conexiones
Locks prolongados
Replicación rota
Redis sin password escuchando fuera de localhost
Postgres/MySQL expuesto públicamente
Base de datos sin backups detectables
Disco de datos casi lleno
```

Importante: credenciales.

No guardes passwords en texto plano. Permite:

```text
variables de entorno
socket local
.pgpass existente
mysql_config_editor existente
prompt temporal
integración futura con secret managers
```

---

## 4.10 Seguridad

Este módulo puede convertir SysTUI en algo mucho más útil que un simple monitor.

No debe prometer “cumplimiento CIS completo” al principio. Mejor:

```text
Security posture checks
CIS-inspired checks
Production hardening hints
```

Checks iniciales:

```text
SSH password authentication enabled
SSH root login enabled
Usuarios con sudo
Usuarios sin contraseña
Usuarios con shell interactiva
Puertos sensibles expuestos
Servicios innecesarios activos
Firewall activo/inactivo
Fail2ban activo/inactivo
Intentos SSH fallidos
Cambios recientes en /etc/passwd, /etc/shadow, /etc/sudoers
Binarios SUID sospechosos
Permisos inseguros en rutas críticas
Certificados próximos a expirar
Docker privileged containers
Docker socket mounted
```

Panel de seguridad:

```text
Security score: 71/100

High:
- Redis exposed on 0.0.0.0:6379
- SSH password login enabled
- 2 privileged Docker containers

Medium:
- No firewall detected
- 3 sudo users
- Certificate expires in 12 days

Low:
- 14 failed SSH attempts in last hour
```

Acciones:

```text
Ver evidencia
Ver recomendación
Copiar comando sugerido
Aplicar fix solo si es seguro
Marcar como aceptado
Añadir excepción
```

Muy importante: no aplicar hardening automático sin explicar.

Ejemplo:

```text
Finding:
SSH password authentication is enabled.

Impact:
Permite ataques de fuerza bruta si el puerto SSH está expuesto.

Suggested remediation:
Set PasswordAuthentication no in sshd_config and reload sshd.

Action:
[View file] [Copy remediation] [Apply with backup]
```

---

## 4.11 Firewall

Módulo separado o submódulo de seguridad/red.

Soporte progresivo:

```text
ufw
firewalld
nftables
iptables legacy
```

Funcionalidades:

```text
Detectar backend activo
Listar reglas
Ver puertos permitidos
Ver zonas si firewalld
Ver chains si nftables
Detectar reglas conflictivas
Relacionar reglas con puertos abiertos
```

Acciones futuras:

```text
Añadir regla
Eliminar regla
Abrir puerto temporal
Cerrar puerto
Backup de reglas
```

Pero en primeras versiones, mejor solo lectura.

---

## 4.12 Certificados TLS

Muy útil para producción.

Funcionalidades:

```text
Detectar certificados en rutas comunes
Leer certificados configurados en nginx/apache si es posible
Comprobar expiración
Comprobar CN/SAN
Comprobar issuer
Comprobar cadena básica
Comprobar certificado remoto host:443
```

Checks:

```text
Certificado expira en menos de 30 días
Certificado expirado
Certificado no coincide con dominio
Certificado autofirmado
Certificado configurado pero archivo no existe
```

Acciones:

```text
Exportar listado
Avisar en dashboard
Relacionar con nginx/apache
```

---

## 4.13 Backups

Un servidor sin visibilidad de backups está incompleto.

SysTUI no tiene que hacer backups al principio. Tiene que detectar si existen y si parecen sanos.

Funcionalidades:

```text
Detectar scripts de backup en cron
Detectar systemd timers de backup
Detectar restic
Detectar borg
Detectar rclone
Detectar pg_dump
Detectar mysqldump
Detectar snapshots
Ver última ejecución conocida
Ver tamaño de destino
Ver logs relacionados
```

Checks:

```text
No hay backup detectable
Backup no ejecutado recientemente
Backup falla en logs
Destino de backup lleno
Backup en el mismo disco que los datos
```

Futuro:

```text
Crear política de backup guiada
Verificar restore
Integración restic/borg
```

---

## 4.14 Paquetes y actualizaciones

Funcionalidad muy útil para servidores.

Soporte:

```text
apt
dnf
yum
pacman
zypper
apk
```

Funcionalidades:

```text
Detectar gestor de paquetes
Listar paquetes actualizables
Detectar actualizaciones de seguridad si la distro lo permite
Ver kernel instalado vs kernel en ejecución
Ver paquetes instalados recientemente
Buscar paquete
Ver servicios que necesitan restart después de update si está disponible
```

Checks:

```text
Muchas actualizaciones pendientes
Kernel actualizado pero servidor no reiniciado
Paquetes críticos desactualizados
Repositorios rotos
```

Acciones:

```text
Actualizar caché
Mostrar comando recomendado
Ejecutar update con confirmación
```

En producción, por defecto solo lectura.

---

## 4.15 Inventario y perfiles

Esto es necesario para convertirlo en herramienta estándar.

Archivo de configuración:

```toml
[hosts.prod-01]
host = "10.0.0.10"
user = "admin"
port = 22
tags = ["prod", "web"]
read_only = true

[hosts.db-01]
host = "10.0.0.20"
user = "admin"
tags = ["prod", "database"]
```

Funcionalidades:

```text
Host selector
Tags
Grupos
Favoritos
Últimos servidores usados
Modo producción con restricciones
Profiles por entorno
Configuración por módulo
Allowlist de puertos esperados
Allowlist de servicios esperados
```

Diferenciador:

```text
Expected state
```

Ejemplo:

```toml
[policy.prod-web]
expected_open_ports = [22, 80, 443]
expected_services = ["nginx", "docker", "sshd"]
forbidden_services = ["redis", "mongodb"]
disk_warning = 80
disk_critical = 90
```

Así SysTUI no solo dice “hay un puerto abierto”, sino:

```text
Puerto 6379 abierto y no está permitido por la política prod-web.
```

---

## 4.16 Fleet mode

Cuando tengas un host bien soportado, añade modo flota.

Funcionalidades:

```text
Ver muchos servidores
Health score por servidor
Checks concurrentes
Agrupación por tags
Comparar hosts
Detectar drift
Buscar servicio en todos los hosts
Buscar puerto en todos los hosts
Exportar reporte global
```

Ejemplo:

```text
Fleet overview:

prod-01   web       82/100   2 warnings
prod-02   web       91/100   OK
db-01     database  64/100   3 high
vpn-01    edge      73/100   1 high
```

Acciones:

```text
Entrar a host
Ejecutar health check
Comparar contra baseline
Generar reporte
```

No metas ejecución masiva destructiva al principio. Es peligrosa.

---

# 5. Interfaz TUI recomendada

## Navegación principal

Atajos:

```text
q        salir
?        ayuda
/        buscar
Ctrl+k   command palette
Tab      cambiar panel
Enter    entrar
Esc      volver
r        refrescar
e        exportar
a        acciones
f        filtros
```

## Command palette

Esto hará que la app se sienta moderna.

Ejemplo:

```text
Ctrl+k

> restart nginx
> open ports
> failed units
> ssh findings
> docker unhealthy
> export report
```

## Layout base

```text
┌─ SysTUI ─ prod-01 ───────────────────────────────────────┐
│ Dashboard | System | Services | Logs | Net | Docker | Sec │
├───────────────────────────────────────────────────────────┤
│                                                           │
│                    contenido principal                    │
│                                                           │
├───────────────────────────────────────────────────────────┤
│ r refresh | / search | a actions | ? help | q quit         │
└───────────────────────────────────────────────────────────┘
```

## Estados de UI necesarios

Debes diseñar bien estos estados:

```text
Loading
Empty
Error
Permission denied
Partial data
Command running
Action confirmation
Success
Failure
Disconnected
Reconnecting
```

Un error no debe romper la pantalla. Si un módulo falla, la app debe seguir viva.

---

# 6. Fases de desarrollo

## Fase 0 — Diseño técnico y alcance real

Objetivo: cerrar el diseño antes de programar demasiado.

Entregables:

```text
Documento de arquitectura
Lista de módulos v0.1, v0.5, v1.0
Modelo de permisos
Diseño del Transport trait
Diseño de Action trait
Diseño de Collector trait
Estructura del workspace Rust
MockTransport para tests
Primer boceto de UI
```

Decisión importante:

```text
SysTUI no debe ejecutar comandos como strings libres.
```

Evita esto:

```rust
run("systemctl restart nginx")
```

Mejor:

```rust
CommandSpec {
    program: "systemctl",
    args: vec!["restart", "nginx"],
    requires_privilege: true,
}
```

Así reduces inyecciones, errores de quoting y problemas con SSH.

---

## Fase 1 — Núcleo de aplicación

Objetivo: crear la base robusta.

Implementar:

```text
Workspace Rust
CLI con clap
Sistema de configuración
Sistema de perfiles
LocalTransport
MockTransport
CommandSpec
CommandOutput
Errores tipados
Logging interno con tracing
Event bus básico
Estado global de aplicación
Tema TUI
Navegación base
Pantalla dashboard vacía
```

Crates útiles:

```text
ratatui
crossterm
tokio
serde
serde_json
toml
chrono
regex
thiserror
anyhow
tracing
clap
```

Criterio de finalización:

```text
systui abre una TUI estable
puede ejecutar collectors mockeados
puede cargar config
puede mostrar errores sin crashear
tiene tests unitarios básicos
```

---

## Fase 2 — Dashboard local mínimo

Objetivo: tener una primera app útil en local.

Implementar collectors:

```text
Sistema operativo
Kernel
Hostname
Uptime
CPU
RAM
Swap
Load average
Discos
Usuarios conectados
```

Pantalla:

```text
Dashboard
System detail
Refresh manual
Refresh automático configurable
```

Checks:

```text
Disco alto
RAM alta
Swap alta
Load alto
```

Criterio:

```text
systui funciona en local y da una visión rápida del sistema
```

Esta será tu primera demo pública.

---

## Fase 3 — Servicios y procesos

Objetivo: que SysTUI empiece a servir para operar.

Implementar:

```text
Listado de procesos
Top CPU/RAM
Detalle de proceso
Árbol simple de procesos
Listado de servicios systemd
Servicios fallidos
Detalle de servicio
Logs recientes de servicio
Start/stop/restart con confirmación
Kill process con confirmación
```

Guardrails:

```text
No matar procesos críticos sin confirmación fuerte
No reiniciar servicios en modo read-only
Mostrar preview de acción
Registrar acción en auditoría
```

Criterio:

```text
desde SysTUI puedo encontrar un servicio fallido, ver sus logs y reiniciarlo de forma segura
```

---

## Fase 4 — Logs potentes

Objetivo: construir uno de los módulos estrella.

Implementar:

```text
journalctl adapter
tail de ficheros
filtros por nivel
filtros por fecha
búsqueda regex
vista por servicio
resumen de errores recientes
agrupación básica por mensaje
exportación de fragmento
```

Detalles importantes:

```text
No cargar logs gigantes enteros en memoria
Lectura incremental
Límites configurables
Backpressure en la UI
```

Criterio:

```text
puedo abrir logs de nginx, sshd o systemd, filtrarlos y detectar errores repetidos
```

---

## Fase 5 — Red y exposición

Objetivo: añadir valor operativo y de seguridad.

Implementar:

```text
Interfaces
IPs
Rutas
DNS
Puertos abiertos
Proceso asociado a puerto
Conexiones activas
Ping
DNS lookup
TCP connect test
Exposure map
```

Checks:

```text
Puertos sensibles expuestos
Servicios escuchando en 0.0.0.0
DNS no funcional
Gateway no accesible
```

Criterio:

```text
SysTUI puede decir qué está expuesto en el servidor y qué proceso lo expone
```

Este módulo es clave para que la herramienta guste a sysadmins y gente de ciberseguridad.

---

## Fase 6 — Seguridad inicial

Objetivo: añadir postura de seguridad útil sin prometer auditoría completa.

Implementar checks:

```text
SSH root login
SSH password authentication
Usuarios con sudo
Failed SSH logins
Firewall detectado
Puertos sensibles
SUID binaries básicos
Permisos de ficheros críticos
Docker socket si existe
Certificados próximos a expirar
```

Modelo de findings:

```text
Finding {
    id,
    severity,
    title,
    evidence,
    impact,
    recommendation,
    related_module,
}
```

Severidades:

```text
Critical
High
Medium
Low
Info
```

Criterio:

```text
SysTUI genera una lista priorizada de riesgos con evidencia y recomendaciones
```

---

## Fase 7 — Docker

Objetivo: cubrir servidores modernos.

Implementar:

```text
Contenedores running/stopped
Stats
Logs
Puertos
Volúmenes
Redes
Restart/start/stop
Health status
Inspect resumido
```

Checks:

```text
Privileged containers
docker.sock montado
Puertos sensibles publicados
Contenedores unhealthy
Restart loop
Tag latest
Sin límites de memoria
```

Criterio:

```text
puedo revisar un servidor Docker y encontrar contenedores problemáticos sin usar varios comandos distintos
```

---

## Fase 8 — Crons y systemd timers

Objetivo: cubrir automatizaciones.

Implementar:

```text
Crontabs por usuario
/etc/crontab
/etc/cron.d
systemd timers
Validación de expresión
Preview de próximas ejecuciones
Crear cron
Editar cron
Eliminar cron
Backup antes de cambios
```

Checks:

```text
Script inexistente
Sin permisos de ejecución
Sin logging
Cron duplicado
Cron root sospechoso
Timer fallido
```

Criterio:

```text
puedo entender qué tareas programadas existen, cuándo corren y si están rotas
```

---

## Fase 9 — SSH remoto

Objetivo: convertir SysTUI en herramienta real de administración.

Implementar:

```text
SshTransport
Host profiles
Known hosts
Autenticación con clave
Autenticación con agente SSH
Puerto personalizado
Timeouts
Reconnect
Ejecución remota de comandos
Lectura remota de ficheros
Detección de permisos
Modo read-only remoto
```

UX:

```text
systui ssh user@host
systui ssh prod-01
systui --profile production
```

Criterio:

```text
todo lo que funciona en local debe funcionar en remoto si el usuario tiene permisos
```

Decisión técnica importante:

Puedes empezar con una implementación basada en OpenSSH del sistema para acelerar compatibilidad, pero si quieres mantener de forma estricta la idea de binario único sin depender del cliente `ssh`, deberías diseñar el transporte para poder migrar a una implementación SSH nativa en Rust.

Lo importante es que el resto de la app no se entere.

---

## Fase 10 — Bases de datos

Objetivo: añadir administración operativa de servicios críticos.

Orden recomendado:

```text
PostgreSQL
Redis
MySQL/MariaDB
MongoDB
```

Implementar primero:

```text
Detección de servicio
Puerto
Estado
Logs
Conexiones
Tamaño
Errores recientes
Exposición de red
```

Luego añadir queries específicas.

Criterio:

```text
SysTUI no reemplaza a psql/mysql-cli, pero permite detectar problemas operativos en segundos
```

---

## Fase 11 — Reportes

Objetivo: que SysTUI sea útil para documentación, auditorías y handover.

Formatos:

```text
JSON
Markdown
HTML
PDF en fase posterior
```

Reportes:

```text
Health report
Security report
Open ports report
Docker report
Failed services report
Host inventory report
Fleet report
```

Ejemplo:

```bash
systui report --host prod-01 --format markdown
systui report --host prod-01 --security --format html
```

Contenido:

```text
Resumen ejecutivo
Estado general
Findings críticos
Evidencias
Servicios fallidos
Puertos expuestos
Contenedores problemáticos
Recomendaciones
Acciones ejecutadas
```

Criterio:

```text
puedo entregar un reporte útil después de revisar un servidor
```

---

## Fase 12 — Fleet mode

Objetivo: pasar de herramienta de servidor individual a herramienta de operación de infraestructura.

Implementar:

```text
Host inventory
Tags
Grupos
Health check concurrente
Vista global
Búsqueda global
Comparación entre hosts
Drift detection básico
Exportación global
```

Ejemplos:

```bash
systui fleet
systui fleet --tag prod
systui fleet check --security
```

Criterio:

```text
puedo ver el estado de 10-50 servidores sin entrar uno por uno
```

No implementes acciones masivas destructivas todavía. Solo inspección y reportes.

---

## Fase 13 — Políticas y estado esperado

Objetivo: convertir SysTUI en una herramienta más inteligente.

Implementar:

```text
Policies por host/grupo
Puertos esperados
Servicios esperados
Servicios prohibidos
Umbrales por entorno
Usuarios permitidos con sudo
Certificados esperados
Contenedores permitidos
```

Ejemplo:

```toml
[policies.production]
disk_warning = 80
disk_critical = 90
expected_ports = [22, 80, 443]
forbidden_ports = [6379, 27017, 9200]
expected_services = ["sshd", "nginx"]
forbidden_services = ["telnet", "vsftpd"]
```

Esto permite detectar drift:

```text
[HIGH] prod-01 expone Redis en 0.0.0.0:6379, pero no está permitido por policy production.
[MED] prod-02 no tiene nginx activo, aunque policy production lo exige.
```

Criterio:

```text
SysTUI deja de ser solo observabilidad y pasa a validar configuración esperada
```

---

## Fase 14 — Packaging y distribución

Objetivo: que instalar SysTUI sea trivial.

Distribuciones:

```text
Binario estático Linux x86_64
Binario estático Linux aarch64
Arch/AUR
.deb
.rpm
Homebrew/Linuxbrew opcional
Cargo install
Docker image opcional para CI/reportes
```

Comandos:

```bash
curl -fsSL https://.../install.sh | sh
cargo install systui
paru -S systui
```

También deberías ofrecer:

```text
Checksums
Firmas
SBOM
Release notes
Changelog
```

Criterio:

```text
un usuario puede instalarlo en menos de un minuto
```

---

## Fase 15 — Estabilización v1.0

Objetivo: calidad real de producto.

Necesario:

```text
Tests unitarios
Tests de integración
Fixtures de comandos
Golden files para parsers
Pruebas en contenedores por distro
Pruebas en VM para systemd real
Fuzzing básico de parsers
Benchmarks de logs grandes
Revisión de seguridad
Documentación completa
Man page
Ejemplos
Demo GIF
```

Matriz mínima:

```text
Ubuntu LTS
Debian stable
Arch
Fedora
Rocky/Alma
Alpine parcial
```

Criterio:

```text
SysTUI es estable, documentado, instalable y usable por terceros sin que tú estés delante explicándolo
```

---

# 7. Funcionalidades diferenciales para que pueda ser “estándar”

Estas son las que más pueden separar SysTUI de herramientas existentes.

## 7.1 Health score explicable

No basta con decir:

```text
Health: 72/100
```

Debe explicar por qué:

```text
-10 disk / above 85%
-8 failed systemd units
-5 SSH password auth enabled
-5 Docker container unhealthy
```

## 7.2 Exposure map

Vista clara de puertos expuestos, proceso, servicio y riesgo.

Esto es extremadamente útil para sysadmin, DevOps y seguridad.

## 7.3 Action safety engine

Antes de tocar nada:

```text
qué se va a hacer
por qué
riesgo
comando equivalente
backup si aplica
confirmación fuerte
auditoría
```

## 7.4 Evidence-based findings

Cada alerta debe mostrar evidencia.

Mal:

```text
SSH is insecure.
```

Bien:

```text
SSH password login is enabled.

Evidence:
/etc/ssh/sshd_config contains:
PasswordAuthentication yes

Risk:
Allows brute-force attempts if SSH is exposed.

Recommendation:
Disable password authentication and use key-based login.
```

## 7.5 Expected state policies

Esto permite que la herramienta sea útil en producción.

No todos los servidores son iguales. Un puerto 5432 puede ser correcto en `db-01`, pero crítico en `web-01`.

## 7.6 Reportes bonitos

Un buen reporte Markdown/HTML puede hacer que la herramienta se use en empresas, auditorías internas, homelabs y documentación.

## 7.7 Fleet mode agentless

Si puedes revisar muchos hosts sin instalar agentes, tienes mucho valor.

## 7.8 Command palette

Hace que la TUI sea rápida.

```text
Ctrl+k → “failed units”
Ctrl+k → “open ports”
Ctrl+k → “restart nginx”
Ctrl+k → “security report”
```

## 7.9 Read-only production mode

Para servidores críticos:

```toml
[hosts.prod-01]
read_only = true
```

Aunque el usuario pulse restart, SysTUI debe bloquearlo.

## 7.10 Session notes

Durante una revisión, permitir añadir notas:

```text
[NOTE] Revisado nginx, errores vienen de upstream api-02.
```

Luego salen en el reporte.

---

# 8. Backlog priorizado por versión

## v0.1 — Demo funcional

```text
TUI base
Dashboard local
CPU/RAM/disk/load
Procesos top
Servicios fallidos
Logs básicos journald
```

Objetivo: enseñar algo visual y útil.

## v0.2 — Operación local

```text
Servicios systemd completos
Procesos completos
Logs con filtros
Acciones seguras
Auditoría local
Modo read-only
```

Objetivo: ya sirve en tu máquina o servidor local.

## v0.3 — Red y seguridad

```text
Puertos abiertos
Exposure map
SSH checks
Firewall detection
Failed login detection
Certificados
Findings con severidad
```

Objetivo: empieza a ser diferencial.

## v0.4 — Docker y crons

```text
Docker containers
Docker logs/stats
Docker risks
Crons
Systemd timers
Validación de crons
```

Objetivo: cubre servidores reales modernos.

## v0.5 — SSH remoto

```text
Remote mode
Profiles
Known hosts
Timeouts
Reconnect
Permisos
Misma UI local/remoto
```

Objetivo: ya es una herramienta real de administración.

## v0.6 — Reportes

```text
JSON
Markdown
HTML
Security report
Health report
Host inventory report
```

Objetivo: útil para documentación y auditoría.

## v0.7 — Bases de datos

```text
PostgreSQL
Redis
MySQL básico
MongoDB básico
Checks de exposición y estado
```

Objetivo: cubrir servicios críticos.

## v0.8 — Fleet

```text
Inventario de hosts
Tags
Vista global
Health concurrente
Búsqueda global
Reportes globales
```

Objetivo: administrar varios servidores.

## v0.9 — Policies

```text
Expected ports
Expected services
Forbidden services
Thresholds
Drift detection
Exceptions
```

Objetivo: SysTUI empieza a validar entornos.

## v1.0 — Release estable

```text
Packaging completo
Documentación
Tests por distro
CI/CD
Firmas
Changelog
Man page
Demo
Página web
```

Objetivo: producto presentable públicamente.

---

# 9. Modelo de datos recomendado

Entidades centrales:

```text
Host
Profile
Snapshot
Metric
Service
Process
Port
Connection
LogEvent
Finding
Action
ActionResult
Report
Policy
Exception
```

Ejemplo conceptual:

```rust
struct Finding {
    id: String,
    severity: Severity,
    title: String,
    description: String,
    evidence: Vec<Evidence>,
    recommendation: String,
    module: ModuleId,
    host: HostId,
    status: FindingStatus,
}
```

Estados de finding:

```text
Open
Accepted
Ignored
Fixed
FalsePositive
```

Esto es importante para que la app no repita eternamente avisos aceptados.

---

# 10. Sistema de acciones

Todas las acciones deben pasar por un motor común.

```text
Action request
  ↓
Permission check
  ↓
Read-only check
  ↓
Risk classification
  ↓
Preview
  ↓
Confirmation
  ↓
Backup if needed
  ↓
Execute
  ↓
Verify
  ↓
Audit log
  ↓
UI result
```

Ejemplo de acción:

```text
RestartService {
    service: "nginx.service",
    risk: Medium,
    requires_privilege: true,
    reversible: false,
}
```

No mezcles acciones dentro de la UI. La UI solo solicita acciones. El motor decide si se pueden ejecutar.

---

# 11. Configuración recomendada

Ruta:

```text
~/.config/systui/config.toml
~/.local/share/systui/
~/.cache/systui/
```

Ejemplo:

```toml
[general]
default_refresh_seconds = 3
theme = "dark"
confirm_dangerous_actions = true
audit_log = true

[ui]
show_health_score = true
compact_mode = false

[security]
ssh_failed_login_window_minutes = 60
cert_expiry_warning_days = 30

[thresholds]
disk_warning = 80
disk_critical = 90
ram_warning = 85
load_warning_multiplier = 1.5

[hosts.prod-01]
host = "192.168.1.20"
user = "admin"
port = 22
tags = ["prod", "web"]
read_only = true
policy = "production-web"

[policies.production-web]
expected_ports = [22, 80, 443]
forbidden_ports = [3306, 5432, 6379, 27017]
expected_services = ["sshd", "nginx"]
```

---

# 12. Testing imprescindible

No puedes construir una app robusta de administración de sistemas sin tests fuertes.

## Tests unitarios

```text
Parsers de systemctl
Parsers de journalctl
Parsers de ss/netstat
Parsers de crontab
Parsers de docker
Severity scoring
Policy evaluation
```

## Tests con fixtures

Guarda salidas reales de comandos:

```text
fixtures/ubuntu/systemctl-list-units.txt
fixtures/arch/ss-tulpn.txt
fixtures/debian/journalctl-nginx.txt
fixtures/fedora/firewalld.txt
```

## Tests de integración

Usa contenedores para:

```text
Debian
Ubuntu
Arch
Fedora
Alpine
```

Para systemd real probablemente necesitarás VMs o contenedores privilegiados específicos.

## Tests de seguridad

```text
No ejecutar acciones en read-only
No permitir shell injection
No aceptar nombres de servicios maliciosos
No registrar secretos en logs
No romper al no tener permisos
```

Casos peligrosos a probar:

```text
service = "nginx; rm -rf /"
service = "../../../etc/passwd"
regex inválida
logs gigantes
SSH timeout
host caído
comando sin permisos
salida en idioma distinto
```

---

# 13. Errores que debes evitar

## Error 1: intentar hacerlo todo desde el principio

Primero haz núcleo, dashboard, servicios, logs, red y seguridad. Docker, crons, DB y fleet después.

## Error 2: acoplar UI y lógica

La UI no debe saber cómo ejecutar `systemctl`. Debe pedir una acción.

## Error 3: depender de parsear texto sin tests

Vas a parsear muchas salidas de comandos. Sin fixtures por distro, la app romperá rápido.

## Error 4: no pensar en permisos

Muchos datos requieren root. La app debe funcionar parcialmente aunque no tenga permisos.

Debe mostrar:

```text
Partial data: permission denied reading /var/log/auth.log
```

No crashear.

## Error 5: convertirlo en herramienta peligrosa

Un TUI con botones de restart, kill y delete puede ser peligroso. Necesitas confirmaciones fuertes, read-only, auditoría y backup.

## Error 6: hacer solo métricas

CPU/RAM/disco no es suficiente. El valor está en correlacionar:

```text
servicio fallido → logs → proceso → puerto → riesgo → acción
```

---

# 14. Orden realista de implementación

Si yo tuviera que construirlo, lo haría en este orden exacto:

```text
1. Workspace Rust
2. CLI
3. Config
4. LocalTransport
5. MockTransport
6. TUI shell
7. Dashboard local
8. System collectors
9. Process collectors
10. Systemd collectors
11. Logs journald
12. Action engine
13. Read-only mode
14. Audit log
15. Network collectors
16. Exposure map
17. Security findings
18. Docker
19. Crons/timers
20. SSH transport
21. Profiles
22. Reports
23. Database checks
24. Fleet mode
25. Policies
26. Packaging
```

La razón de retrasar SSH hasta después de varios módulos es técnica: si primero haces bien local + abstracción de transporte, SSH será una implementación más. Si empiezas por SSH, se te contaminará toda la arquitectura.

---

# 15. Versión inicial que deberías publicar

Tu primera release pública no debería decir “suite completa”. Debería venderse así:

```text
SysTUI v0.1
A fast Linux server health TUI focused on services, logs and exposure.
```

Debe incluir:

```text
Dashboard
CPU/RAM/disk/load
Failed services
Service logs
Top processes
Open ports
Basic security findings
Markdown report
```

Eso ya es suficiente para generar interés real.

Luego publicas posts tipo:

```text
“Built a Rust TUI to detect exposed services and failed systemd units in seconds”
“SysTUI: agentless Linux server health dashboard in your terminal”
“Finding risky open ports from a TUI”
```

---

# 16. Definición de SysTUI v1.0

Para considerar la app completa y robusta, v1.0 debería tener:

```text
Local mode estable
Remote SSH mode estable
Dashboard inteligente
System metrics
Processes
Systemd services
Logs
Network exposure map
Docker
Crons/timers
Security findings
Certificates
Package/update checks
Reports JSON/Markdown/HTML
Profiles
Read-only mode
Audit log
Action safety engine
Policy checks básicos
Packaging Linux
Documentación completa
Tests por distro principal
```

Lo que dejaría para v1.1/v2:

```text
Fleet avanzado
Remediaciones automáticas complejas
Kubernetes
Plugin SDK público
Agente opcional
Alerting continuo
Integraciones con Vault/Bitwarden/1Password
PDF avanzado
Web dashboard
```

---

# 17. Núcleo conceptual del producto

La app debería girar alrededor de esta idea:

```text
Detect → Explain → Correlate → Act safely → Report
```

Ejemplo:

```text
Detect:
nginx has 7 recent upstream timeout errors.

Explain:
The service is active, but logs show repeated upstream timeouts.

Correlate:
nginx listens on 443 and proxies to 127.0.0.1:3000.
Process on 3000 is using 94% CPU.

Act safely:
Show process, show logs, optionally restart backend service.

Report:
Include finding, evidence, action taken and result.
```

Eso es lo que puede convertir SysTUI en una herramienta seria. No una pantalla bonita, sino una consola operativa que reduce tiempo de diagnóstico.