# Derleme ve dagitim

## Karar

**Flatpak birincil dagitim bicimi, yerel derleme ikincil.**

Gerekce tek tabloda duruyor: GTK4 ve libadwaita surumleri dagitimlar arasinda
dort yil aciyor. Ortak paydaya inmek (GTK 4.6 / libadwaita 1.1) `OverlaySplitView`,
`ToolbarView`, `PreferencesDialog`, `SwitchRow` ve `SpinRow`'u masadan kaldirir —
yani planin yarisini. Runtime'i sabitlemek bu problemi tumuyle ortadan kaldiriyor.

Yerel derleme icin taban **Ubuntu 24.04 LTS**: GTK 4.14 / libadwaita 1.5 / VTE 0.76.
Bu ucluyu `Cargo.toml`'daki `v4_14` / `v1_5` / `v0_76` ozellik bayraklari ve
`src/config.rs`'teki calisma ani kontrolu birlikte zorunlu kiliyor.

## Dagitim matrisi

| Dagitim | GTK4 | libadwaita | vte-gtk4 | Yerel derleme | Flatpak |
| --- | --- | --- | --- | --- | --- |
| Arch / CachyOS / EndeavourOS | 4.24 | 1.10 | 0.84 | tam | tam |
| Fedora 40+ | 4.14+ | 1.5+ | var | tam | tam |
| Ubuntu 24.04 LTS (noble) | 4.14.5 | 1.5.0 | 0.76.0 | tam (taban) | tam |
| Ubuntu 25.04+ | 4.18+ | 1.7+ | 0.78+ | tam | tam |
| Debian 13 (trixie) | 4.18.6 | 1.7.6 | 0.80.1 | tam | tam |
| Debian 12 (bookworm) | 4.8.3 | 1.2.2 | 0.70.6 | **hayir** | tam |
| Ubuntu 22.04 LTS (jammy) | 4.6.9 | 1.1.7 | yok | **hayir** | tam |
| Rocky / RHEL 10 | 4.16 | 1.6 | yok | yalniz kabuk `*` | tam |
| Rocky / RHEL 9 | 4.6 | yok/eski | yok | **hayir** | tam |
| openSUSE Tumbleweed | guncel | guncel | var | tam | tam |

`*` Rocky 10'da GTK ve libadwaita yetiyor ama `vte291-gtk4` paketi yok;
`--no-default-features` ile terminalsiz kabuk derlenir. Terminal icin Flatpak sart.

Matristeki surumler dagitimlarin kendi paket arsivlerinden dogrulandi
(Launchpad, packages.debian.org). Bunlarin altinda kalan her sey Flatpak.

## Flatpak (her dagitim)

```bash
flatpak install -y flathub org.gnome.Platform//50 org.gnome.Sdk//50 \
    org.freedesktop.Sdk.Extension.rust-stable//25.08
pip install --user aiohttp toml tomlkit   # flatpak-cargo-generator icin
curl -LO https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
python3 flatpak-cargo-generator.py Cargo.lock -o build-aux/cargo-sources.json
flatpak-builder --user --install --force-clean build build-aux/dev.runetty.Runetty.yaml
flatpak run dev.runetty.Runetty
```

Runtime GNOME 50'ye sabitli: her dagitimda ayni GTK 4.20 / libadwaita 1.8.
VTE manifest icinde 0.78.4 olarak derleniyor, dagitimin surumune bakilmiyor.

## Yerel derleme

```bash
./build-aux/install-deps.sh     # pacman / apt / dnf / zypper algilar
cargo build --release
./target/release/runetty
```

Betik kurulumun sonunda `pkg-config` surumlerini tabanla karsilastirir ve
yetmiyorsa Flatpak'a yonlendirir.

### vte-gtk4 olmayan sistemler

```bash
cargo build --release --no-default-features
```

Terminal sekmeleri yerine bir aciklama etiketi gelir; arayuzun geri kalani calisir.
Rocky 10 ve benzeri sistemlerde gorsel uzerinde calismak icin yeterli.

### Elle paket listeleri

| Aile | Paketler |
| --- | --- |
| Arch | `base-devel pkgconf rust gtk4 libadwaita vte4 libsecret inter-font ttf-jetbrains-mono` |
| Debian / Ubuntu | `build-essential pkg-config libgtk-4-dev libadwaita-1-dev libvte-2.91-gtk4-dev libsecret-1-dev fonts-inter fonts-jetbrains-mono` |
| Fedora / RHEL | `gcc pkgconf-pkg-config gtk4-devel libadwaita-devel libsecret-devel vte291-gtk4-devel` |
| openSUSE | `gcc pkg-config gtk4-devel libadwaita-devel vte-gtk4-devel libsecret-devel` |

### Rust surumu

**Rust 1.92 ya da ustu gerekiyor** (gtk4-sys'in MSRV'si). Debian, Ubuntu ve RHEL'in
paket `rustc`'si bunun altinda kaldigi icin paket listelerinde yok — rustup kullanin:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup default stable
```

Arch'ta dagitim paketi guncel; Flatpak'te `rust-stable` SDK eklentisi hallediyor.

## Parolalar

Kaydedilen parolalar sistem keyring'ine (Secret Service) yazilir — GNOME Keyring
ya da KDE'de KWallet. `hosts.toml`'a ya da baska bir dosyaya duz metin parola
yazilmaz. Baglanirken parola ssh'a `SSH_ASKPASS` uzerinden verilir, yani komut
satirinda hicbir zaman gorunmez ve `sshpass` gibi bir araca gerek kalmaz.

Keyring calismiyorsa (ornegin kwallet/gnome-keyring kapaliysa) parola kaydedilemez;
uygulama bunu soyler ve ssh parolayi her zamanki gibi terminalde sorar.

## Yazi tipleri

Arayuz Inter, kimlikler JetBrains Mono. Ikisi de yoksa `style.css`'teki yedek
zincir devreye girer (Adwaita Sans / Cantarell, DejaVu Sans Mono) — uygulama
calisir ama tipografik ikilik zayiflar. Flatpak surumu host'un fontlarini gorur;
font paketlerini dagitimdan kurmak yeterli.

## Gorsel dogrulama

```bash
./target/debug/runetty --screenshot /tmp/runetty.png
```

Pencereyi offscreen render edip PNG'ye basar ve cikar. Faz 1'in gecme olcutu
("iki ekran goruntusu yan yana ayirt edilemiyor") bununla olculur.

```bash
./target/debug/runetty --version    # gtk ve libadwaita calisma ani surumleri
```
