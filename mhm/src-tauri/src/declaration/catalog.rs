//! Danh mục enum và hành chính, sinh từ template chính thức của cổng.
//!
//! Không hardcode và không viết tay: `mhm/scripts/gen_kbtt_catalog.py` đọc
//! `tblt_vn_import.xlsx` rồi xuất `kbtt_catalog.json`. Cổng ra template mới thì
//! chạy lại script, `git diff` chỉ ra đúng cái gì đổi.

use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogItem {
    pub code: String,
    pub display: String,
    #[serde(default)]
    pub tinh: String,
}

#[derive(Debug, Clone, Copy)]
pub enum CatalogList {
    LoaiGiayTo,
    NoiCuTru,
    LyDoCuTru,
    GioiTinh,
    QuocTich,
    TinhThanh,
    PhuongXa,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    #[serde(rename = "_source_sha256")]
    pub source_sha256: String,
    #[serde(rename = "_source_date")]
    pub source_date: String,
    pub loai_giay_to: Vec<CatalogItem>,
    pub noi_cu_tru: Vec<CatalogItem>,
    pub ly_do_cu_tru: Vec<CatalogItem>,
    pub gioi_tinh: Vec<CatalogItem>,
    pub quoc_tich: Vec<CatalogItem>,
    pub tinh_thanh: Vec<CatalogItem>,
    pub phuong_xa: Vec<CatalogItem>,
}

impl Catalog {
    pub fn load() -> Result<Catalog, String> {
        let path = super::find_kbtt_resource("kbtt_catalog.json")?;
        Catalog::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Result<Catalog, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("Không đọc được {}: {e}", path.display()))?;
        serde_json::from_str(&raw).map_err(|e| format!("kbtt_catalog.json hỏng: {e}"))
    }

    fn list(&self, which: CatalogList) -> &[CatalogItem] {
        match which {
            CatalogList::LoaiGiayTo => &self.loai_giay_to,
            CatalogList::NoiCuTru => &self.noi_cu_tru,
            CatalogList::LyDoCuTru => &self.ly_do_cu_tru,
            CatalogList::GioiTinh => &self.gioi_tinh,
            CatalogList::QuocTich => &self.quoc_tich,
            CatalogList::TinhThanh => &self.tinh_thanh,
            CatalogList::PhuongXa => &self.phuong_xa,
        }
    }

    /// Chuỗi ghi vào file. Nguyên văn từ template — không bao giờ tự ghép lại
    /// từ code + tên, vì sai một dấu cách là cổng không nhận mà không báo.
    pub fn display_for(&self, which: CatalogList, code: &str) -> Option<&str> {
        self.list(which)
            .iter()
            .find(|i| i.code == code)
            .map(|i| i.display.as_str())
    }

    pub fn has_code(&self, which: CatalogList, code: &str) -> bool {
        self.list(which).iter().any(|i| i.code == code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat() -> Catalog {
        Catalog::load().expect("catalog phải nạp được")
    }

    #[test]
    fn catalog_has_expected_counts() {
        let c = cat();
        assert_eq!(c.loai_giay_to.len(), 9);
        assert_eq!(c.noi_cu_tru.len(), 3);
        assert_eq!(c.ly_do_cu_tru.len(), 20);
        assert_eq!(c.gioi_tinh.len(), 2);
        // 198, KHÔNG phải 205: named range QUOC_TICH khai E2:E206 nhưng
        // E26-E32 trống — template thiếu 7 nước giữa Botswana và Cameroon.
        assert_eq!(c.quoc_tich.len(), 198);
        assert_eq!(c.tinh_thanh.len(), 34);
        assert_eq!(c.phuong_xa.len(), 3323);
    }

    #[test]
    fn display_is_verbatim_from_template() {
        let c = cat();
        assert_eq!(
            c.display_for(CatalogList::LoaiGiayTo, "1"),
            Some("1 - Thẻ CCCD")
        );
        assert_eq!(
            c.display_for(CatalogList::LoaiGiayTo, "8"),
            Some("8 - Thẻ Căn Cước")
        );
        assert_eq!(c.display_for(CatalogList::GioiTinh, "F"), Some("F - Nữ"));
        assert_eq!(
            c.display_for(CatalogList::LyDoCuTru, "1"),
            Some("1 - Du lịch")
        );
        assert_eq!(
            c.display_for(CatalogList::LyDoCuTru, "20"),
            Some("20 - Mục đích khác")
        );
    }

    #[test]
    fn nationality_lookup_reflects_the_template_hole() {
        let c = cat();
        assert!(c.has_code(CatalogList::QuocTich, "VNM"));
        assert!(c.has_code(CatalogList::QuocTich, "RUS"));
        assert!(!c.has_code(CatalogList::QuocTich, "ZZZ"));
        // Cổng thiếu thật — không phải lỗi của ta, nhưng phải biết
        assert!(!c.has_code(CatalogList::QuocTich, "BRA"));
    }

    #[test]
    fn ward_to_province_relation_is_present() {
        let c = cat();
        let nha_trang: Vec<_> = c.phuong_xa.iter().filter(|w| w.tinh == "511").collect();
        assert!(!nha_trang.is_empty(), "Khánh Hòa 511 phải có phường");
    }

    #[test]
    fn source_provenance_recorded() {
        let c = cat();
        assert_eq!(c.source_sha256.len(), 64);
        assert!(!c.source_date.is_empty());
    }
}
