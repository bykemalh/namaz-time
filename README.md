# Namaz Vakti (`me.bykemah.namaztime`)

GNOME masaüstü ortamı için **GTK4**, **Libadwaita** ve **Rust** ile geliştirilmiş yerel namaz vakitleri ve sistem tepsisi (AppIndicator) uygulaması.

- **Depo:** https://github.com/bykemalh/namaz-time
- **Geliştirici:** Kemal ([@bykemalh](https://github.com/bykemalh))
- **Lisans:** MIT ([COPYING](COPYING))

## Özellikler

- **GNOME / Libadwaita Arayüzü:** Aktif vakti, bir sonraki vakte kalan canlı geri sayımı (`SS:DD:SN` veya `SS:DD`), vakit ilerleme çubuğunu, Miladi/Hicri tarihi ve 6 günlük vakti modern Adwaita kartlarıyla gösterir.
- **Sistem Tepsisi (GNOME Top Bar) & DBusMenu:**
  - Tepsi ikonuna tek tıklandığında konum özeti, kalan süre, tüm 6 vakit ve hızlı işlemler (**Uygulamayı Göster**, **Vakitleri Yenile**, **Tercihler**, **Çıkış**) menüde görünür.
  - GNOME üst çubuğunda ikonun yanında kalan süreyi gösterme (`Vakit + Süre` veya `Yalnızca Süre`) Tercihler'den açılıp kapatılabilir.
- **Arka Planda Çalışma & Otomatik Başlatma:** Pencere kapatıldığında uygulama tepside arka planda çalışmaya devam eder; oturum açılışında XDG Background Portalı ile otomatik başlatılabilir.
- **3 Aşamalı Doğrulamalı Diyanet Veri Kaynağı & Yerel Önbellek:**
  1. **İmsakiyem API** (birincil; Türkiye ve dünya geneli)
  2. **mtopal Ezan Vakti v1.5** (yedek; Türkiye ve dünya geneli, IANA saat dilimiyle yerel hesaplama)
  3. **İDA Medya** (ek yedek; yalnızca Türkiye)
  - Başarısız isteklerde isim/ilçe doğrulamasıyla sıradaki kaynağa geçer, bağlantı olmadığında yerel XDG önbellekten (`~/.cache/me.bykemah.namaztime`) çalışmayı sürdürür.

## Sistem Gereksinimleri

- GNOME üst panelinde tepsi ikonunu ve kalan süre etiketini görmek için **AppIndicator and KStatusNotifierItem Support** GNOME Shell eklentisinin etkin olması gerekir.

## Flatpak Paketleme & Yayınlama

### 1. Gerekli Flatpak SDK ve Araçları Kurun

```bash
sudo dnf install flatpak-builder
flatpak install --user flathub \
  org.gnome.Platform//50 \
  org.gnome.Sdk//50 \
  org.freedesktop.Sdk.Extension.rust-stable//25.08
```

*(Rust uzantı dalı yüklü `org.gnome.Sdk//50` ile uyumlu değilse `flatpak install flathub org.freedesktop.Sdk.Extension.rust-stable` komutuyla uyumlu sürümü seçebilirsiniz.)*

### 2. Tek Dosya `.flatpak` Paketi Oluşturun (GitHub Releases İçin)

Manifest ([me.bykemah.namaztime.json](me.bykemah.namaztime.json)), [cargo-sources.json](cargo-sources.json) sayesinde tüm Rust bağımlılıklarını ağ erişimi olmadan çevrimdışı (offline) derleyecek şekilde yapılandırılmıştır:

```bash
flatpak-builder --user --force-clean --repo=repo build-dir me.bykemah.namaztime.json
flatpak build-bundle repo namaz-time-0.1.0-x86_64.flatpak me.bykemah.namaztime
```

Üretilen `.flatpak` dosyasını kurup test etmek için:

```bash
flatpak install --user ./namaz-time-0.1.0-x86_64.flatpak
flatpak run me.bykemah.namaztime
```

### 3. Yerel Meson + `org.gnome.Sdk//50` İle Derleme

```bash
flatpak run --filesystem="$PWD" --command=sh org.gnome.Sdk//50 -c '
  meson setup _build --prefix=/app &&
  meson test -C _build &&
  meson compile -C _build
'
```

### 4. GitHub'a Yükleme ve Flathub Notları

```bash
git add .
git commit -m "Initial release v0.1.0"
git branch -M main
git remote add origin https://github.com/bykemalh/namaz-time.git
git push -u origin main
```

Flathub başvurusu yaparken [me.bykemah.namaztime.json](me.bykemah.namaztime.json) içindeki `"type": "dir"` kaynağını etiketlenmiş Git kaynağıyla değiştirmeniz gerekir:

```json
{
    "type": "git",
    "url": "https://github.com/bykemalh/namaz-time.git",
    "tag": "v0.1.0",
    "commit": "<git-commit-sha>"
}
```

> **Not:** Flathub mağazası App ID için alan adı sahipliği doğrulaması ister (`bykemah.me` alan adı size ait değilse Flathub gönderiminde `io.github.bykemalh.NamazTime` kimliğini kullanmanız gerekebilir; GitHub Releases üzerinden dağıtılan `.flatpak` paketlerinde `me.bykemah.namaztime` doğrudan çalışır). Ayrıca `Cargo.lock` güncellendiğinde `cargo-sources.json` dosyasını `flatpak-cargo-generator Cargo.lock -o cargo-sources.json` ile yeniden üretebilirsiniz.

## Veri Kaynakları

- [İmsakiyem API](https://ezanvakti.imsakiyem.com) (Diyanet verisi)
- [mtopal Ezan Vakti API v1.5](https://ezanvakti.emushaf.net) (Diyanet verisi)
- [İDA Medya Namaz Vakti API](https://api.idamedya.com) (Türkiye / Diyanet verisi)
