# ferrentd Debian package

Build (with the real engine):

```bash
cargo deb -p ferrent-daemon --features libtorrent
```

The package installs:

- `/usr/bin/ferrentd`
- `/lib/systemd/system/ferrentd.service`
- `/etc/ferrent/config.toml` (conffile, `0640 root:ferrent`)
- `/usr/share/man/man1/ferrentd.1.gz`

`postinst` creates the `ferrent` system user/group, `/var/lib/ferrent`, and a
random API token in `/etc/ferrent/token` (`0640 root:ferrent`). The token is
never part of the package contents.

`apt remove ferrentd` keeps `/etc/ferrent` and `/var/lib/ferrent`;
`apt purge ferrentd` removes `/var/lib/ferrent` and the generated token.
`postrm` only deletes the token on purge; the config remains a dpkg conffile.

The unit uses `FERRENT_CONFIG=/etc/ferrent/config.toml`, waits up to 120 s for
a clean shutdown (fastresume save), and limits the service to
`/var/lib/ferrent` writes under `ProtectSystem=strict`.
