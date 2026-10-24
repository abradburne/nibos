#!/bin/sh
# Start-up trimming for the Arch root file system: run on a root directory
# (make-arch.sh) or on the device itself (tune-boot.sh / USER).
#
#   tune-boot.sh ROOT USER      USER: the tty1 autologin user, who lingers
set -eu
root=${1:?usage: tune-boot.sh ROOT USER}
user=${2:?usage: tune-boot.sh ROOT USER}

# Generators run before systemd loads any units, each one a program read from
# the card. Keep fstab, getty, debug and run; the rest have nothing to do here
# (no TPM, encryption, verity, hibernation, GPT auto-mounts or cloud metadata).
mkdir -p "$root/etc/systemd/system-generators"
for g in bless-boot clonesetup cryptsetup factory-reset gpt-auto hibernate-resume \
         imds import integritysetup ssh system-update tpm2 veritysetup; do
    ln -sf /dev/null "$root/etc/systemd/system-generators/systemd-$g-generator"
done

# The console logs in by itself, so it needn't wait for the network: upstream,
# getty waits for systemd-user-sessions, which waits for network.target (iwd),
# only to remove /run/nologin. Don't create /run/nologin, and drop that wait.
mkdir -p "$root/etc/tmpfiles.d"
ln -sf /dev/null "$root/etc/tmpfiles.d/systemd-nologin.conf"
#
# getty is also Type=idle, which holds agetty back until every other start-up
# job is done (2 s here) so boot messages don't land on the prompt; with a quiet
# boot there are none, so start it as soon as it's due.
sed -e 's/^After=systemd-user-sessions.service /After=/' -e 's/^Type=idle$/Type=simple/' \
    "$root/usr/lib/systemd/system/getty@.service" > "$root/etc/systemd/system/getty@.service"
# The user manager, which logind makes the console's login wait for, is also
# ordered after systemd-user-sessions, so the same network wait was still there.
sed '/^After=/s/ network.target//' "$root/usr/lib/systemd/system/systemd-user-sessions.service" \
    > "$root/etc/systemd/system/systemd-user-sessions.service"

# The console font is the kernel's (fbcon=font:TER16x32), and the keymap is US.
ln -sf /dev/null "$root/etc/systemd/system/systemd-vconsole-setup.service"

# Coldplug only devices. Upstream also replays an "add" for every driver, module
# and bus (911 events here, ~1.35 s of udevd CPU while sysinit runs); the rules
# that look at those are for hardware this device lacks or modules that send a
# real event when they load. Device classes no rule matches are skipped too.
mkdir -p "$root/etc/systemd/system/systemd-udev-trigger.service.d"
cat > "$root/etc/systemd/system/systemd-udev-trigger.service.d/nibos.conf" <<'UNIT'
[Service]
ExecStart=
ExecStart=-udevadm trigger --type=devices --action=add --prioritized-subsystem=module,block,tpmrm,net,tty,input \
    --subsystem-nomatch=devlink --subsystem-nomatch=dma --subsystem-nomatch=workqueue \
    --subsystem-nomatch=clockevents --subsystem-nomatch=clocksource --subsystem-nomatch=wakeup \
    --subsystem-nomatch=genpd_provider --subsystem-nomatch=regulator --subsystem-nomatch=serial-base \
    --subsystem-nomatch=memory --subsystem-nomatch=bdi
UNIT

# The vconsole rule restarts the (masked) vconsole service for each vtcon: two
# failing systemctl runs during coldplug.
mkdir -p "$root/etc/udev/rules.d"
ln -sf /dev/null "$root/etc/udev/rules.d/90-vconsole.rules"

# No credentials are ever passed in (no firmware, no VM), so don't run the two
# loaders; the udev one sits right before udevd.
for u in systemd-udev-load-credentials systemd-userdb-load-credentials; do
    ln -sf /dev/null "$root/etc/systemd/system/$u.service"
done

# Arch's environment generator only prints PATH, but it's a shell started before
# any unit; set the same PATH directly.
mkdir -p "$root/etc/systemd/system-environment-generators" "$root/etc/systemd/system.conf.d"
ln -sf /dev/null "$root/etc/systemd/system-environment-generators/10-arch"
cat > "$root/etc/systemd/system.conf.d/nibos-path.conf" <<'CONF'
[Manager]
DefaultEnvironment=PATH=/usr/local/sbin:/usr/local/bin:/usr/bin
CONF

# Arch merges every group found in /etc/group with systemd's user database, so
# each group lookup is two varlink calls, one of them to PID 1. Early units make
# hundreds (tmpfiles alone 122) while PID 1 is busy starting units. Ask systemd
# only for groups /etc/group doesn't have (dynamic users still resolve).
sed -i 's/^group: *files \[SUCCESS=merge\] systemd/group: files systemd/' "$root/etc/nsswitch.conf"

# systemd-backlight@ is ordered before sysinit.target, so everything after
# sysinit (dbus, logind, the console) waited until PID 1 had worked through the
# coldplug backlog to the backlight devices, ~0.7 s after udev finished them.
# Brightness is restored whenever they appear; nothing needs it before then.
sed 's/^Before=sysinit.target /Before=/' "$root/usr/lib/systemd/system/systemd-backlight@.service" \
    > "$root/etc/systemd/system/systemd-backlight@.service"

# With no initrd, journald finds /var/log/journal at start and flushes on its
# own; the flush unit's request then waits ~0.7 s on a busy journald, and
# tmpfiles-setup (so sysinit.target) waited for it. Keep the unit (its stop
# step lets go of /var at shutdown) but don't order tmpfiles after it.
sed '/^Before=systemd-tmpfiles-setup.service$/d' "$root/usr/lib/systemd/system/systemd-journal-flush.service" \
    > "$root/etc/systemd/system/systemd-journal-flush.service"

# resolved (~0.7 s to start here) and timesyncd, which waits for it, are ordered
# before sysinit.target, so everything after sysinit waited for them. Nothing
# that early resolves names or needs synced time: they stay before
# network.target, nss-lookup.target and time-set.target, which is what users of
# names and time wait for.
#
# The same for udevd, ~0.5 s to start while coldplug loads it (devices appear
# whenever it's up; nothing before the console waits for one), and for the utmp
# boot record.
for u in systemd-resolved systemd-timesyncd systemd-udevd systemd-update-utmp; do
    sed -E '/^Before=/{s/(=| )sysinit\.target( |$)/\1/;/^Before=$/d;}' "$root/usr/lib/systemd/system/$u.service" \
        > "$root/etc/systemd/system/$u.service"
done

# Hugepages and the FUSE control file system have no users here; two fewer
# mounts in the early burst.
for u in dev-hugepages sys-fs-fuse-connections; do
    ln -sf /dev/null "$root/etc/systemd/system/$u.mount"
done

# tmpfiles runs over /dev twice: "early" (--graceful, for users that don't
# exist yet) and again once sysusers is done. The users all exist here, so the
# second pass only adds a step to the console's path.
ln -sf /dev/null "$root/etc/systemd/system/systemd-tmpfiles-setup-dev.service"

# sshd starts per connection from a socket instead of at boot, where it took
# ~1 s of CPU while the console's login was running. Arch's sshd.service can't
# take a listening socket from systemd, so this is its sshd@.service (sshd -i)
# behind an Accept=yes socket on port 22 (IPv4 and IPv6).
cat > "$root/etc/systemd/system/sshd.socket" <<'UNIT'
[Unit]
Description=OpenSSH Server Socket
Documentation=man:sshd(8)
Before=ssh-access.target
Wants=ssh-access.target

[Socket]
ListenStream=22
Accept=yes

[Install]
WantedBy=sockets.target
UNIT
rm -f "$root/etc/systemd/system/multi-user.target.wants/sshd.service"
mkdir -p "$root/etc/systemd/system/sockets.target.wants"
ln -sf ../sshd.socket "$root/etc/systemd/system/sockets.target.wants/sshd.socket"

# The console's login waited ~1.2 s for the user manager: logind holds a "user"
# session until user@UID is up, and starting it went through a PID 1 that is
# still busy booting. Instead the user lingers, so logind starts user@UID (audio,
# the user bus) when it starts, and tty logins are "user-light" sessions, which
# don't wait for it. The login stack also drops what the autologin doesn't need:
# lastlog2's SQLite write, motd, mail, systemd-homed, pam_access and pam_time.
mkdir -p "$root/var/lib/systemd/linger"
touch "$root/var/lib/systemd/linger/$user"
cat > "$root/etc/pam.d/login" <<'PAM'
#%PAM-1.0
# nibos (rootfs/tune-boot.sh): tty logins, trimmed for the tty1 autologin.
# A user-light session doesn't wait for the user manager; the autologin user
# lingers, so its user manager (audio, the user bus) starts with the system.
auth       requisite    pam_nologin.so
auth       include      system-local-login
account    required     pam_unix.so
password   include      system-local-login
session    optional     pam_loginuid.so
session    optional     pam_keyinit.so       force revoke
session    required     pam_limits.so
session    required     pam_unix.so
session    optional     pam_umask.so
session    optional     pam_systemd.so       class=user-light
session    required     pam_env.so
PAM
