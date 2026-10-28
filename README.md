# runetty

SSH oturumlari, agent'lar ve sunucu izleme icin GTK4 + libadwaita masaustu
istemcisi. Gorsel dil [docs/plan.md](docs/plan.md)'de; teknik taban ve dagitim matrisi
[BUILDING.md](BUILDING.md)'de.

## Durum

Faz 1 — gorsel iskelet.

- [x] Tasarim sistemi tek dosyada (`data/resources/style.css`), Rust'ta renk yok
- [x] Pencere iskeleti: ikon rayi + `OverlaySplitView` + sekmeli icerik
- [x] Kenar cubugu: Recents / Servers, klasor agaci, iki katmanli satirlar
- [x] Bos durum: komut alani, "Jump back in" cipleri, klavye ipucu satiri
- [x] Agent paneli: durum noktasi + hap rozet
- [x] VTE terminal sekmeleri (`--no-default-features` ile kapatilabilir)
- [x] Cok dagitimli derleme: Flatpak + dort paket yoneticisi + CI
- [ ] Komut paleti (Ctrl K)
- [ ] `~/.ssh/config` okuma
- [ ] Monitor sayfasi ve halka gostergeler
- [ ] Ayar diyalogu

## Hizli baslangic

```bash
./build-aux/install-deps.sh
cargo build --release
./target/release/runetty
```

Dagitiminiz tabanin altindaysa betik sizi Flatpak'a yonlendirir; ayrintilar
[BUILDING.md](BUILDING.md)'de.

## Lisans

MIT. GTK4/libadwaita (LGPL-2.1) ve VTE (LGPL-3.0) dinamik baglaniyor;
Inter ve JetBrains Mono OFL.

Tasarim dili SigilTTY'den ilham aliyor; ad, logo ve ikon cizimi kendimizin.
