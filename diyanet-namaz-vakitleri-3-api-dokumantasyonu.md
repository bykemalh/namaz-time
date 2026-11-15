# Diyanet Verilerine Dayanan 3 Ücretsiz Namaz Vakti API'si

> **Güncelleme / kontrol tarihi:** 15 Kasım 2026  
> **Kapsam:** İDA Medya, mtopal Namaz Vakti API v1.5, İmsakiyem v1 + v2  
> **Dil:** Türkçe  
> **Amaç:** Üç bağımsız API kaynağını aynı uygulamada kullanmak ve gerektiğinde birbirine yedeklemek.

## 0. Önemli açıklamalar

- Bu üçü **resmî Diyanet API'si değildir**. Diyanet kaynaklı namaz vakti verilerini sunan üçüncü taraf servislerdir.
- Üçü de **API key/oturum açma gerektirmeyen** halka açık GET çağrıları sunar; ücretsiz erişim sınırsız kullanım garantisi değildir.
- Diyanet'in resmî, kimlik doğrulamalı sistemi ayrıca: <https://awqatsalah.diyanet.gov.tr/>.
- Bu belgedeki **endpoint ve parametreler yayımlanan dokümanlara göre** kontrol edildi. İDA Medya `/namaz/iller` ve `/namaz/9541`, mtopal `/ulkeler` yanıtı web üzerinden ayrıca görüldü. **Bütün örnek isteklerin uçtan uca çalıştığı garanti edilmez**; özellikle İmsakiyem v2 canlı yanıtları bu kontrolde doğrulanamadı. Uygulamaya almadan önce kendi ortamınızda test edin.
- **Kritik: İlçe kimliklerini servisler arasında körlemesine eşitlemeyin.** IDA Medya'daki `9541` sorgusunun `yer` alanı *İstanbul* dönerken mtopal dokümanı `9541` için *Kadıköy* örneğini gösteriyor. Fallback kurarken her sağlayıcının konumunu **şehir/ilçe adı ile doğrulayın**.
- Namaz saatleri `HH:mm` metnidir; yerel tarihle ve **gerçek yerel saat dilimiyle** birlikte saklayın. `HH:mm` tek başına UTC zamanı değildir.

## 1. Hızlı karşılaştırma

| Özellik | İDA Medya | mtopal v1.5 | İmsakiyem v1 / v2 |
|---|---|---|---|
| API kökü | `https://idamedya.org/api/v1` | `https://namazvakti.mtopal.dev` | `https://ezanvakti.imsakiyem.com` |
| API key | Yok | Yok | Yok |
| Yöntem | GET | GET (HEAD da desteklenir) | GET |
| Yanıt | JSON, `ok/data` | JSON dizi | JSON, `success/data` |
| Konum arama | İl → ilçe | Ülke → şehir → ilçe | v1 hiyerarşi, v2 arama / koordinat |
| Vakit kapsama | Yaklaşık bir ay | Bugünden yıl sonuna | Günlük/haftalık/aylık/yıllık/aralık |
| İstek sınırlaması | Sayısal limit açıklanmamış; aşırı istek engellenebilir | IP ve endpoint grubu başına 10/dk + 100/saat | v1: 100 istek / 5 dk; v2 limiti ayrıca teyit edilmeli |
| Tarayıcı CORS | Açık (dokümana göre) | `*` (dokümana göre) | Tarayıcıda ayrıca test edilmeli |

---

# 2. API 1: İDA Medya

**Dokümantasyon:** <https://idamedya.org/api/>  
**Base URL:** `https://idamedya.org/api/v1`  
**İşlev:** Türkiye'deki iller/ilçeler ve yakın dönem namaz vakitleri.

## 2.1. Endpoint listesi

| Yöntem | Yol | Açıklama |
|---|---|---|
| GET | `/namaz/iller` | 81 ilin adı, il merkezi vakit ID'si ve ilçe listesinin şehir ID'si |
| GET | `/namaz/ilceler/{sehir}` | Belirtilen ilin ilçeleri ve vakit kimlikleri |
| GET | `/namaz/{ilce}` | İlgili vakit kodu için bugünden yaklaşık bir ay namaz vakti |

## 2.2. İstek örnekleri

```bash
# İl listesi
curl -sS 'https://idamedya.org/api/v1/namaz/iller'

# İstanbul ilçeleri (şehir ID=539)
curl -sS 'https://idamedya.org/api/v1/namaz/ilceler/539'

# Vakit kodu 9541 (canlı örnekte "yer": "İstanbul")
curl -sS 'https://idamedya.org/api/v1/namaz/9541'
```

### Parametreler

- `{sehir}`: `/namaz/iller` öğesindeki `sehir` değeri. Örneğin İstanbul `539`.
- `{ilce}`: İl merkezinin `/namaz/iller` öğesindeki `ilce` değeri **veya** `/namaz/ilceler/{sehir}` yanıtındaki uygun vakit kodu.
- Bu API'nin namaz vakti endpoint'inde belirli tarih veya `period` parametresi belgelenmemiştir.

## 2.3. Gerçek yanıt örnekleri

**GET `/namaz/iller` — dizi içindeki bir öğe**

```json
{
  "ok": true,
  "data": [
    { "ad": "İstanbul", "ilce": 9541, "sehir": 539 }
  ]
}
```

> Yukarıdaki `data` dizisi yalnızca **tek öğe göstermek için kısaltılmıştır**; gerçek yanıt 81 il içerir.

**GET `/namaz/9541` — gerçek yanıtından tek güne indirgenmiş örnek**

```json
{
  "ok": true,
  "data": {
    "yer": "İstanbul",
    "ilce": 9541,
    "kaynak": "Diyanet İşleri Başkanlığı",
    "gunler": [
      {
        "tarih": "2026-11-15",
        "hicri": "6 Cemaziyelahir 1448",
        "imsak": "06:16",
        "gunes": "07:44",
        "ogle": "12:54",
        "ikindi": "15:29",
        "aksam": "17:53",
        "yatsi": "19:16"
      }
    ]
  }
}
```

> Gerçek istek tek gün değil, birden fazla günü döndürür. Üstteki örnek **15 Kasım 2026 anlık içeriğinden kısaltmadır**; dinamik vakit olarak kod içine sabitlenmemelidir.

**Başarı zarfı:** `{ "ok": true, "data": ... }`  
**Hata zarfı:** `{ "ok": false, "error": "Açıklama" }`

## 2.4. Önbellekleme ve erişim

- Dokümantasyon namaz vakitleri için yaklaşık **1 saat** önbelleklemeyi önerir.
- CORS açık olduğu belirtilir.
- Aşırı istekler engellenebilir; net bir dakika/saat kotası yayımlanmamış.
- Kaynak belirtin: İDA Medya / Diyanet kaynaklı vakit verisi.

---

# 3. API 2: mtopal Namaz Vakti API v1.5

**Ana sayfa:** <https://namazvakti.mtopal.dev/>  
**Dokümantasyon:** <https://namazvakti.mtopal.dev/docs.html>  
**Base URL:** `https://namazvakti.mtopal.dev`  
**Kapsam (sağlayıcı beyanı):** 209 ülke, 365 şehir, 18.647 ilçe.

## 3.1. Endpoint listesi

| Yöntem | Yol | Açıklama | Alternatif query biçimi |
|---|---|---|---|
| GET | `/ulkeler` | Ülkeler | — |
| GET | `/sehirler/{ulke_id}` | Ülkenin şehirleri | `/sehirler?ulke={id}` |
| GET | `/ilceler/{sehir_id}` | Şehrin ilçeleri | `/ilceler?sehir={id}` |
| GET | `/vakitler/{ilce_id}` | Bugünden yıl sonuna namaz vakitleri | `/vakitler?ilce={id}` |

## 3.2. Türkiye → İstanbul → Kadıköy akışı

```bash
# 1) Ülkeler
curl -sS 'https://namazvakti.mtopal.dev/ulkeler'

# 2) Türkiye (UlkeID=2) şehirleri
curl -sS 'https://namazvakti.mtopal.dev/sehirler/2'

# 3) İstanbul (SehirID=539) ilçeleri
curl -sS 'https://namazvakti.mtopal.dev/ilceler/539'

# 4) Dokümana göre Kadıköy örneği (IlceID=9541)
curl -sS 'https://namazvakti.mtopal.dev/vakitler/9541'
```

## 3.3. JSON alanları

**Ülke:**

```json
{
  "UlkeAdi": "TÜRKİYE",
  "UlkeAdiEn": "TURKEY",
  "UlkeID": "2"
}
```

**Şehir:** `SehirAdi`, `SehirAdiEn`, `SehirID`, `BagliOlduguUlkeID`  
**İlçe:** `IlceAdi`, `IlceAdiEn`, `IlceID`, `BagliOlduguSehirID`

**Namaz vakti:** Endpoint doğrudan **bir JSON dizisi** döndürür. Dokümantasyondaki tarihsel örnekten bir öğe:

```json
{
  "Imsak": "05:32",
  "Gunes": "07:01",
  "Ogle": "13:18",
  "Ikindi": "16:42",
  "Aksam": "19:22",
  "Yatsi": "20:43",
  "MiladiTarihKisa": "18.03.2026",
  "MiladiTarihKisaIso8601": "18.03.2026",
  "MiladiTarihUzun": "18 Mart 2026",
  "MiladiTarihUzunIso8601": "2026-03-18T00:00:00.0000000+03:00",
  "HicriTarihKisa": "18 Ramazan 1447",
  "HicriTarihUzun": "18 Ramazan 1447",
  "GreenwichOrtalamaZamani": 3
}
```

> Bunlar dokümantasyonun **eski tarihli örnek saatleridir**; güncel saat olarak kullanmayın.

## 3.4. Limit, HTTP hataları ve özel notlar

- **10 istek/dakika**, **100 istek/saat** (IP başına ve endpoint grubu başına).
- `429 Too Many Requests`: limit aşıldı.
- Diğer belgelenmiş kodlar: `404` bulunamadı, `405` yanlış yöntem, `422` eksik parametre, `502` kaynak hatası.
- Hata gövdesi: `{ "detail": "Hata açıklaması." }`.
- CORS başlığı: `Access-Control-Allow-Origin: *`.
- Vakitler yıl sonuna kadar sağlanır, sonraki yıla geçişte yeniden veri çekilmelidir.
- **Önemli veri sorunu:** `MiladiTarihUzunIso8601` içindeki `+03:00` sağlayıcının açıklamasına göre diğer ülkelerde de Türkiye ofsetidir. Yurt dışı saat dilimi hesabında doğrudan kullanmayın.
- Ülke isimlerinde yazım, aksan ve İngilizce karşılıklar tutarsız olabilir.

---

# 4. API 3: İmsakiyem Ezan Vakti API

**Ana sayfa / açıklamalar:** <https://ezanvakti.imsakiyem.com/>  
**Swagger:** <https://ezanvakti.imsakiyem.com/api-docs/>  
**OpenAPI JSON:** <https://ezanvakti.imsakiyem.com/api-docs/json>  
**Kaynak kodu:** <https://github.com/karademirmustafa/ezanvakti-imsakiyem-api>  
**Base URL:** `https://ezanvakti.imsakiyem.com`  
**OpenAPI sürümü:** 1.1.0 (15 Kasım 2026 taramasında görülen).

## 4.1. v1 — Konum endpoint'leri

| Yöntem | Endpoint | Parametre |
|---|---|---|
| GET | `/api/locations/countries` | Yok |
| GET | `/api/locations/countries/{id}` | Ülke ID'si |
| GET | `/api/locations/states?countryId={id}` | Ülke ID'si |
| GET | `/api/locations/states/{id}` | İl ID'si |
| GET | `/api/locations/districts?stateId={id}` | İl ID'si |
| GET | `/api/locations/districts/{id}` | İlçe ID'si |
| GET | `/api/locations/search/{type}?q={text}` | `type` = `countries`, `states`, `districts` |

```bash
# Ülke listesi
curl -sS -H 'Accept: application/json' \
  'https://ezanvakti.imsakiyem.com/api/locations/countries'

# Türkiye'nin illeri (countryId=2)
curl -sS 'https://ezanvakti.imsakiyem.com/api/locations/states?countryId=2'

# Ankara ilçeleri (stateId=506)
curl -sS 'https://ezanvakti.imsakiyem.com/api/locations/districts?stateId=506'

# İsim arama
curl -sS 'https://ezanvakti.imsakiyem.com/api/locations/search/districts?q=Polatli'
```

## 4.2. v1 — Namaz vakitleri

**Endpoint:** `GET /api/prayer-times/{districtId}/{period}`

| Parametre | Tip | Açıklama |
|---|---|---|
| `districtId` | Path / string | İlçe ID |
| `period` | Path / enum | `daily`, `weekly`, `monthly`, `yearly` (OpenAPI'de bu dört değer var) |
| `startDate` | Query / tarih | `YYYY-MM-DD`, opsiyonel |
| `endDate` | Query / tarih | `YYYY-MM-DD`, opsiyonel |
| `page` | Query / int | Sayfa, varsayılan 1 (README) |
| `limit` | Query / int | Sayfa boyutu, varsayılan 100 (README) |

> Ana sayfa README belgesinde `range` kelimesi de geçiyor; fakat **15 Kasım 2026 OpenAPI enum listesinde `range` bulunmuyor**. Aralık için `startDate`/`endDate` parametreleri veya v2 `from`/`to` tercih edin.

```bash
# Dokümantasyonda kullanılan örnek konum: Polatlı (9220)
curl -sS 'https://ezanvakti.imsakiyem.com/api/prayer-times/9220/daily'
curl -sS 'https://ezanvakti.imsakiyem.com/api/prayer-times/9220/weekly'
curl -sS 'https://ezanvakti.imsakiyem.com/api/prayer-times/9220/monthly'
curl -sS 'https://ezanvakti.imsakiyem.com/api/prayer-times/9220/yearly'

# Tarih verilerek aylık sorgu
curl -sS 'https://ezanvakti.imsakiyem.com/api/prayer-times/9220/monthly?startDate=2026-11-15'

# Sayfalama
curl -sS 'https://ezanvakti.imsakiyem.com/api/prayer-times/9220/yearly?page=1&limit=100'
```

**v1 OpenAPI yanıt şeması — önemli alanlar:**

```json
{
  "success": true,
  "code": 200,
  "message": "Namaz vakitleri başarıyla alındı.",
  "data": [
    {
      "date": "YYYY-MM-DD",
      "times": {
        "imsak": "HH:mm",
        "gunes": "HH:mm",
        "ogle": "HH:mm",
        "ikindi": "HH:mm",
        "aksam": "HH:mm",
        "yatsi": "HH:mm"
      }
    }
  ],
  "pagination": {
    "currentPage": 1,
    "totalPages": 1,
    "totalItems": 1,
    "itemsPerPage": 100
  }
}
```

> **Şematik örnek**: `YYYY-MM-DD`, `HH:mm` gerçek veri değil; alanların yerini gösterir. `data[]` öğelerinde `districtId`, `hijri_date` vb. alanlar da bulunabilir. `pagination` bazı isteklerde farklı olabilir.

**Belgelenmiş v1 kota:** 100 istek / 5 dakika. Hatalar: 400, 404, 429, 500.

## 4.3. v2 — Konum endpoint'leri

v2 yeni `api/v2` yollarını kullanır. v1 yolları korunmuştur.

| Endpoint | Açıklama | Temel parametreler |
|---|---|---|
| `GET /api/v2/locations/countries` | Ülkeler | Yok |
| `GET /api/v2/locations/countries/{id}` | Ülke detayı | `id` |
| `GET /api/v2/locations/districts` | Filtreli ilçeler, sayfalama | `countryId`, `stateId`, `region`, `q`, `page`, `limit` |
| `GET /api/v2/locations/districts/{id}` | İlçe detayları, koordinat/saat dilimi | `id` |
| `GET /api/v2/locations/search` | İlçe adına göre arama | `q` (en az 2 karakter), `country`, `limit` |
| `GET /api/v2/locations/nearest` | Koordinata yakın ilçeler | `lat`, `lon`, `radius_km`, `limit` |

**v2 konum parametreleri**

| Parametre | Aralık / varsayılan |
|---|---|
| `page` (`districts`) | Varsayılan 1 |
| `limit` (`districts`) | 1–500, varsayılan 50 |
| `limit` (`search`) | 1–50, varsayılan 10 |
| `lat` | -90..90 |
| `lon` | -180..180 |
| `radius_km` | 1–500 km, varsayılan 50 |
| `limit` (`nearest`) | 1–20, varsayılan 5 |

```bash
# Ülkeler
curl -sS 'https://ezanvakti.imsakiyem.com/api/v2/locations/countries'

# Ülke detayı
curl -sS 'https://ezanvakti.imsakiyem.com/api/v2/locations/countries/2'

# Ankara ilçeleri
curl -sS 'https://ezanvakti.imsakiyem.com/api/v2/locations/districts?stateId=506&page=1&limit=50'

# İlçe araması (aksan duyarsız)
curl -sS 'https://ezanvakti.imsakiyem.com/api/v2/locations/search?q=polatli&limit=10'

# Ankara koordinatlarının yakınındaki ilçe kayıtları
curl -sS 'https://ezanvakti.imsakiyem.com/api/v2/locations/nearest?lat=39.93&lon=32.85&radius_km=50&limit=5'
```

## 4.4. v2 — Namaz vakitleri endpoint'leri

| Endpoint | Açıklama |
|---|---|
| `GET /api/v2/times` | Günlük, haftalık, aylık, yıllık veya aralıklı vakitler |
| `GET /api/v2/times/next` | Mevcut ve sıradaki namaz; kalan saniye |

**`GET /api/v2/times` parametreleri**

| Parametre | Zorunluluk | Açıklama |
|---|---|---|
| `district` | Konum seçiminin bir yolu | İlçe kimliği |
| `lat`, `lon` | `district` yoksa birlikte | Koordinat; en yakın ilçeyi bulur |
| `radius_km` | Hayır | Yakın ilçe arama yarıçapı; varsayılan 50 |
| `date` | Hayır | `YYYY-MM-DD`; yoksa ilçenin yerel bugünü |
| `period` | Hayır | `daily` (varsayılan), `weekly`, `monthly`, `yearly` |
| `from` ve `to` | Hayır, ancak birlikte | Özel aralık; en fazla 366 gün |

**Kural:** `district` ile `lat`/`lon` birlikte kullanılmaz. Sadece tarih gönderip konum göndermemek hatadır.

```bash
# İlçe ID ile bugün
curl -sS 'https://ezanvakti.imsakiyem.com/api/v2/times?district=9220'

# İlçe ID ile belirli gün
curl -sS 'https://ezanvakti.imsakiyem.com/api/v2/times?district=9220&date=2026-11-15'

# Belirli ay
curl -sS 'https://ezanvakti.imsakiyem.com/api/v2/times?district=9220&period=monthly&date=2026-11-15'

# Aralık
curl -sS 'https://ezanvakti.imsakiyem.com/api/v2/times?district=9220&from=2026-11-15&to=2026-11-30'

# Koordinatla bugün
curl -sS 'https://ezanvakti.imsakiyem.com/api/v2/times?lat=39.93&lon=32.85'

# Sıradaki vakit + kalan saniye
curl -sS 'https://ezanvakti.imsakiyem.com/api/v2/times/next?district=9220'

# Koordinatla sıradaki vakit
curl -sS 'https://ezanvakti.imsakiyem.com/api/v2/times/next?lat=39.93&lon=32.85'
```

**v2 yanıt zarfı (OpenAPI şemasından):**

```json
{
  "success": true,
  "data": {},
  "meta": { "version": "2" }
}
```

`data` endpoint'e göre **nesne veya dizi** olabilir; `/api/v2/times` için belgede `times` alanının **daima dizi** olduğu yazılıdır. Gerçek `data` nesnesini canlı test edip kodda doğrulayın.

**v2 hata zarfı (şematik):**

```json
{
  "success": false,
  "error": {
    "code": "DISTRICT_NOT_FOUND",
    "message": "Açıklama"
  },
  "meta": { "version": "2" }
}
```

Sık görülebilecek hata kodları: `VALIDATION_ERROR`, `LOCATION_REQUIRED`, `AMBIGUOUS_LOCATION`, `INVALID_DATE`, `RANGE_TOO_LARGE`, `DISTRICT_NOT_FOUND`, `NO_DISTRICT_NEARBY`, `NO_TIMES_FOR_RANGE`.

İstek kimliği v2'de `x-request-id` yanıt başlığında bulunur. Hata kaydında saklanabilir.

## 4.5. Açık yıllık veri kümesi

İmsakiyem ayrıca Türkiye ve KKTC için yıllık JSON arşivi yayımlıyor. Büyük dosyalar GitHub Releases sayfasından alınabiliyor. API kesintisinde statik yedek olarak değerlendirilebilir.

- Doküman: <https://ezanvakti.imsakiyem.com/open-data/prayer-times/README.md>
- GitHub: <https://github.com/karademirmustafa/ezanvakti-imsakiyem-api>
- Her yılın dosyasında hangi yılın olduğu ve kapsadığı bölgeler ayrıca kontrol edilmelidir. Güncellenmeyen eski veri, yeni yıl için kullanılmamalıdır.

---

# 5. Üçünü birlikte kullanma: önerilen tasarım

## 5.1. Genel mimari

```text
React Router / Mobil istemci
           |
           v
Kendi backend'in (Go / Rust / Node.js)
           |
           +--> Veritabanı / Redis / yerel cache
           |
           +--> İmsakiyem v2 (1. tercih: koordinat veya ilçe)
           |
           +--> mtopal v1.5 (2. tercih: ilçe / yıl sonuna kadar)
           |
           +--> İDA Medya (3. tercih: Türkiye / yaklaşık bir ay)
           |
           +--> Son geçerli cache (üçü de erişilemezse)
```

Bu **bir öneridir**, servis sağlayıcılarının resmî öncelik sırası değildir. Kullanıcıya istersen kaynağı manuel seçtirebilirsin.

## 5.2. Lokasyon eşleme (en kritik adım)

Bir şehir veya ilçe için **üç ayrı sağlayıcı ID'si** sakla. Örnek şema:

```json
{
  "country": "TR",
  "city": "İstanbul",
  "district": "Kadıköy",
  "timezone": "Europe/Istanbul",
  "providerIds": {
    "idamedya": null,
    "mtopal": "9541",
    "imsakiyem": null
  }
}
```

`null` olanlar **henüz doğrulanmadı** anlamına gelir; tahminî ID ile istek atma. İDA Medya'nın `/namaz/iller` yanıtındaki il merkezi vakit koduyla mtopal'ın `IlceID` alanı aynı anlama gelmeyebilir. Aynı kodu kullanarak farklı konumların saatlerini karşılaştırmak yanlış sonuç verir.

Önerilen eşleme adımları:

1. Sağlayıcıların ülke/şehir/ilçe listelerini indir.
2. Unicode/aksan farklarını normalize et; mümkünse ISO ülke kodu ve yerel idari bölge bilgisini kullan.
3. Her kaynakta eşleşen konumun adını ve üst bölgesini kontrol et.
4. Eşlemeyi veritabanında `provider + location` olarak sakla.
5. Konum eşleşmesi belirsizse fallback yapma; doğru konumu kullanıcıya doğrulat.

## 5.3. Tek biçimli JSON şeması

**Önerilen kendi backend yanıtın** (bu bir sağlayıcının gerçek yanıtı değildir):

```json
{
  "date": "2026-11-15",
  "country": "TR",
  "city": "İstanbul",
  "district": "Kadıköy",
  "timezone": "Europe/Istanbul",
  "source": "mtopal",
  "stale": false,
  "times": {
    "imsak": "06:00",
    "gunes": "07:00",
    "ogle": "12:00",
    "ikindi": "15:00",
    "aksam": "18:00",
    "yatsi": "19:00"
  }
}
```

> Yukarıdaki saatler **yalnızca format örneğidir**, gerçek namaz vakti değildir.

Alan isimleri uygulama boyunca sabit kalsın: `imsak`, `gunes`, `ogle`, `ikindi`, `aksam`, `yatsi`.

## 5.4. Örnek JavaScript veri normalizasyonu

Aşağıdaki yardımcılar **İDA Medya**, **mtopal** ve **İmsakiyem v1** şemalarının belgelenmiş alanlarına dayanır. İmsakiyem v2'yi ayrıca test edip gerçek `data` yapısına göre adapte et.

```js
const HHMM = /^([01]\d|2[0-3]):[0-5]\d$/;
const fields = ['imsak', 'gunes', 'ogle', 'ikindi', 'aksam', 'yatsi'];

function ensureDay(day) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(day.date)) {
    throw new Error('Geçersiz tarih');
  }
  for (const key of fields) {
    if (!HHMM.test(day.times?.[key] ?? '')) {
      throw new Error(`Geçersiz ${key} vakti`);
    }
  }
  return day;
}

function fromIdamedya(body) {
  if (body?.ok !== true || !Array.isArray(body.data?.gunler)) {
    throw new Error('İDA Medya yanıt biçimi beklenmiyor');
  }
  return body.data.gunler.map(row => ensureDay({
    date: row.tarih,
    times: Object.fromEntries(fields.map(k => [k, row[k]]))
  }));
}

function fromMtopal(body) {
  if (!Array.isArray(body)) {
    throw new Error('mtopal yanıt biçimi beklenmiyor');
  }
  const names = {
    imsak: 'Imsak', gunes: 'Gunes', ogle: 'Ogle',
    ikindi: 'Ikindi', aksam: 'Aksam', yatsi: 'Yatsi'
  };
  return body.map(row => {
    const [dd, mm, yyyy] = (row.MiladiTarihKisa ?? '').split('.');
    return ensureDay({
      date: `${yyyy}-${mm}-${dd}`,
      times: Object.fromEntries(fields.map(k => [k, row[names[k]]]))
    });
  });
}

function fromImsakiyemV1(body) {
  if (body?.success !== true || !Array.isArray(body.data)) {
    throw new Error('İmsakiyem v1 yanıt biçimi beklenmiyor');
  }
  return body.data.map(row => ensureDay({
    date: String(row.date).slice(0, 10),
    times: Object.fromEntries(fields.map(k => [k, row.times?.[k]]))
  }));
}
```

### Basit HTTP istemcisi ve yedek kaynak mantığı

```js
async function fetchJSON(url, timeoutMs = 6000) {
  const response = await fetch(url, {
    headers: { Accept: 'application/json' },
    signal: AbortSignal.timeout(timeoutMs)
  });
  if (!response.ok) {
    const error = new Error(`HTTP ${response.status}`);
    error.status = response.status;
    throw error;
  }
  return response.json();
}

// IDs: üç ayrı sağlayıcıda *aynı doğrulanmış konumun* ID'leri.
// Bir ID bilinmiyorsa o sağlayıcıyı atla.
async function getPrayerDays(ids) {
  const providers = [
    {
      source: 'imsakiyem-v1', id: ids.imsakiyem,
      url: id => `https://ezanvakti.imsakiyem.com/api/prayer-times/${encodeURIComponent(id)}/monthly`,
      normalize: fromImsakiyemV1
    },
    {
      source: 'mtopal', id: ids.mtopal,
      url: id => `https://namazvakti.mtopal.dev/vakitler/${encodeURIComponent(id)}`,
      normalize: fromMtopal
    },
    {
      source: 'idamedya', id: ids.idamedya,
      url: id => `https://idamedya.org/api/v1/namaz/${encodeURIComponent(id)}`,
      normalize: fromIdamedya
    }
  ];

  const errors = [];
  for (const p of providers) {
    if (p.id == null) continue;
    try {
      const raw = await fetchJSON(p.url(p.id));
      const days = p.normalize(raw);
      if (!days.length) throw new Error('Boş namaz vakti dizisi');
      return { source: p.source, days, stale: false };
    } catch (error) {
      errors.push(`${p.source}: ${error.message}`);
      // 429 yanıtında bu kaynağa yüklenmeyi durdur;
      // uygun cache / Retry-After bekleme stratejisi uygula.
    }
  }
  throw new Error(`Tüm kaynaklar başarısız: ${errors.join(' | ')}`);
}

// ÖRNEK: ÇALIŞMASI İÇİN ID'LERİN AYNI KONUMA AİT OLDUĞU TEYİT EDİLMELİ.
// const result = await getPrayerDays({
//   imsakiyem: '<DOGRU_IMSAKIYEM_ID>',
//   mtopal: '<DOGRU_MTOPAL_ID>',
//   idamedya: '<DOGRU_IDAMEDYA_ID>'
// });
```

> Bu örnek sadeleştirilmiş **başlangıç kodudur**; canlı servise göre uyarlanmalı. Üretimde günlük kapsama, aynı gün/konum doğrulaması, cache, kayıt/log ve retry sınırları eklenmelidir. `AbortSignal.timeout` güncel Node.js ortamlarında desteklenir.

## 5.5. Önbellek, retry ve veri güvenilirliği

1. **İlçe listeleri:** Nadir güncellenir; 1–7 gün TTL + manuel yenileme düşünülebilir.
2. **Günlük vakitler:** İDA Medya en az 1 saat cache öneriyor. mtopal yıl sonuna kadar döndürdüğü için tarih bazlı uzun süre saklanabilir; yine de düzeltme/güncelleme için periyodik yeniden eşitleme yapın.
3. **Tarih sınırları:** Seçilen gün JSON içinde yoksa diğer kaynağa geçin; "ilk eleman her zaman bugün" varsayımıyla kalıcı cache işlemi yapmayın.
4. **429:** `Retry-After` varsa dikkate alın; yoksa kontrollü backoff yapın. Her istekte üç sağlayıcıya eşzamanlı yüklenmekten kaçının.
5. **5xx/timeout:** Son geçerli veriye geçebilirsiniz, ancak sonucu `stale: true` ile işaretleyin. Geçerli gün için hiçbir doğru veri yoksa saat uydurmayın.
6. **Konum:** Aynı ilçe adı farklı ülkelerde/illerde bulunabilir. Konum eşleşmesini yalnızca isim benzerliğiyle yapmayın.
7. **Saat dilimi:** Türkiye için `Europe/Istanbul`; diğer ülkelerde gerçek IANA saat dilimini belirleyin. mtopal ISO alanındaki `+03:00` değerinin tüm ülkeleri temsil etmediğini unutmayın.
8. **Kontrol:** Gerekli ise üç sağlayıcıdan aynı gün ve konum için saat farklarını karşılaştırıp alarm üretin. Fark varsa otomatik olarak bir saati doğru ilan etmek yerine konum, tarih, saat dilimi ve kaynak güncelliğini denetleyin.
9. **Veri kaynağı:** Kullanıcı arayüzünde üçüncü taraf servis adını ve Diyanet kaynaklı hesaplanan vakit bilgisini ayırarak gösterin.

## 5.6. Test kontrol listesi

- [ ] Her servis için ülke/şehir/ilçe listesi çağrıları HTTP 200 dönüyor.
- [ ] Her serviste aynı **gerçek ilçe** için doğru ID ayrı ayrı bulundu.
- [ ] Günlük vakitler aynı ISO tarihine eşlendi.
- [ ] JSON başarılı/hatalı şekilleri test edildi.
- [ ] Ön yüzde gerekiyorsa CORS kontrol edildi (tercihen kendi backend üzerinden çağır).
- [ ] 404, 429 ve 5xx hatalarıyla fallback test edildi.
- [ ] Cache TTL ve yeni gün/yıl geçişi kontrol edildi.
- [ ] Geçmiş/güncel/gelecek tarih kapsaması doğrulandı.
- [ ] Saat dilimi ve yaz saati uygulaması kontrol edildi.
- [ ] Üç kaynak kapalıyken sistem yanlış saat üretmiyor.

---

# 6. Kaynaklar ve doğrulama

**15 Kasım 2026** itibarıyla erişilen birincil kaynaklar:

1. İDA Medya dokümanı — <https://idamedya.org/api/>
2. İDA Medya canlı il listesi — <https://idamedya.org/api/v1/namaz/iller>
3. İDA Medya canlı vakit yanıtı — <https://idamedya.org/api/v1/namaz/9541>
4. mtopal v1.5 dokümantasyonu — <https://namazvakti.mtopal.dev/docs.html>
5. mtopal canlı ülke listesi — <https://namazvakti.mtopal.dev/ulkeler>
6. İmsakiyem Swagger UI — <https://ezanvakti.imsakiyem.com/api-docs/>
7. İmsakiyem OpenAPI 3.0 JSON (sürüm 1.1.0) — <https://ezanvakti.imsakiyem.com/api-docs/json>
8. İmsakiyem proje README — <https://ezanvakti.imsakiyem.com/>
9. İmsakiyem açık veri dokümanı — <https://ezanvakti.imsakiyem.com/open-data/prayer-times/README.md>
10. İmsakiyem kaynak kodu — <https://github.com/karademirmustafa/ezanvakti-imsakiyem-api>
11. Resmî Diyanet (bu üç servisten ayrı) — <https://awqatsalah.diyanet.gov.tr/>

**Doğrulama sınırı:** Bazı canlı API adresleri otomatik web erişiminde açılamadı. Endpoint açıklamaları, bunlar için ilgili servislerin **yayımlanmış dokümantasyonuna** dayandırıldı; bu durum servislerin kapalı olduğunun kanıtı değildir. Kullanıma almadan önce kendi sunucunuzdan `curl -i` ile HTTP durumunu ve güncel JSON'u teyit edin.
