# Read-ahead at power-on

`nibos-readahead` is a systemd generator: generators run at ~1.4 s from the kernel's start,
before any unit, and this one forks a reader into its own session and returns at once. The
reader reads the files in `/etc/nibos/readahead.list` into the page cache, at idle I/O
priority, in 128 KB reads, so that the launcher (DSlot) and Mesa are in memory when they start.

```bash
rootfs/readahead/install.sh            # build in rgbuild, install the generator and list
rootfs/readahead/install.sh HOST off   # remove
```

The list is DSlot's, up to its first frame (`readahead.list`; the shader cache's files are added
on the device). `nibos-readahead --record LIST` prints which parts of each listed file are in
the page cache, for making a list after a boot without read-ahead. `/run/nibos-readahead`
says when it ran and how much it read. Results: [docs/boot-time.md](../../docs/boot-time.md#the-boot-ui-a-picture-at-15-s-the-shelf-at-52-s).
The list names the DSlot user's home as `@HOME@`; `install.sh` fills it in.
