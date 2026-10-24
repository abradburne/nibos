# nibos-no-fdk-aac

Stands in for `libfdk-aac`, so nibOS's images and devices don't carry it: the Fraunhofer FDK
AAC licence grants no patent rights and restricts more than nibOS passes on. It provides and
conflicts with `libfdk-aac`, and `rootfs/make-arch.sh` keeps PipeWire's Bluetooth AAC plugin
from being unpacked; AAC headphones use SBC (or aptX, LDAC, LC3, Opus where they have them).
More in the [PKGBUILD](PKGBUILD) and [docs/engineering/audio.md](../../docs/engineering/audio.md).

```bash
cd rootfs/no-fdk-aac && makepkg       # in the build machine; arch any
```

On a device set up without it, run as root once:

```bash
sed -i '/^\[options\]/a NoExtract = usr/lib/spa-0.2/bluez5/libspa-codec-bluez5-aac.so' /etc/pacman.conf
rm -f /usr/lib/spa-0.2/bluez5/libspa-codec-bluez5-aac.so
pacman -U --ask=4 nibos-no-fdk-aac-*.pkg.tar.*
systemctl --user restart wireplumber pipewire   # as $NIBOS_USER
```
