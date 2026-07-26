#!/usr/bin/env python3
"""Sinh kbtt_catalog.json từ template chính thức của cổng Bộ Công an.

Chạy lại script này mỗi khi cổng phát hành template mới. git diff sẽ chỉ ra
đúng cái gì đổi — đó là cách biến schema drift thành một thay đổi nhìn thấy
được thay vì một khai báo sai im lặng.

Dùng: python3 mhm/scripts/gen_kbtt_catalog.py
"""

import hashlib
import json
import re
import sys
import zipfile
from datetime import date
from pathlib import Path

MHM = Path(__file__).resolve().parents[1]
TEMPLATE = MHM / "src-tauri/resources/tblt_vn_import.xlsx"
OUT = MHM / "src-tauri/resources/kbtt_catalog.json"

# (khóa json, sheet xml, cột, độ rộng named range, số giá trị thật)
#
# Với `quoc_tich`, hai con số này KHÁC NHAU. Named range QUOC_TICH khai
# E2:E206 = 205 dòng, nhưng E26–E32 trống — template chính thức thiếu hẳn 7
# quốc tịch giữa `BWA - Botswana` và `CMR - Cameroon`, trong đó có Brazil,
# Brunei và Bulgaria. Danh sách dùng được thật sự chỉ có 198 mã.
ENUMS = [
    ("loai_giay_to", "sheet4", "A", 9, 9),
    ("noi_cu_tru", "sheet4", "B", 3, 3),
    ("ly_do_cu_tru", "sheet4", "C", 20, 20),
    ("gioi_tinh", "sheet4", "D", 2, 2),
    ("quoc_tich", "sheet4", "E", 205, 198),
    ("tinh_thanh", "sheet2", "C", 34, 34),
]

WARD_COUNT = 3323


def shared_strings(z):
    xml = z.read("xl/sharedStrings.xml").decode("utf-8")
    return [
        "".join(re.findall(r"<t[^>]*>(.*?)</t>", si, re.S))
        for si in re.findall(r"<si>(.*?)</si>", xml, re.S)
    ]


def cells(z, sheet, strs):
    xml = z.read(f"xl/worksheets/{sheet}.xml").decode("utf-8")
    out = {}
    for m in re.finditer(r'<c r="([A-Z]+\d+)"([^>]*)>(.*?)</c>', xml, re.S):
        ref, attr, inner = m.groups()
        v = re.search(r"<v>(.*?)</v>", inner, re.S)
        if not v:
            continue
        val = v.group(1)
        if 't="s"' in attr:
            val = strs[int(val)]
        out[ref] = val
    return out


def split_code(display):
    """`511 - Khánh Hòa` -> `511`. Cắt ở ' - ' ĐẦU TIÊN."""
    if " - " not in display:
        raise SystemExit(f"Mục danh mục không đúng dạng 'mã - nhãn': {display!r}")
    return display.split(" - ", 1)[0].strip()


def main():
    if not TEMPLATE.exists():
        raise SystemExit(f"Không thấy template: {TEMPLATE}")

    raw = TEMPLATE.read_bytes()
    z = zipfile.ZipFile(TEMPLATE)
    strs = shared_strings(z)

    catalog = {
        "_source_file": TEMPLATE.name,
        "_source_sha256": hashlib.sha256(raw).hexdigest(),
        "_source_date": date.today().isoformat(),
    }

    gaps = {}
    for key, sheet, col, span, want in ENUMS:
        data = cells(z, sheet, strs)
        # Quét hết độ rộng named range rồi BỎ QUA ô trống, thay vì dừng ở ô
        # trống đầu tiên. Template có lỗ ở giữa danh sách.
        items = []
        blanks = []
        for row in range(2, 2 + span):
            val = data.get(f"{col}{row}")
            if not val:
                blanks.append(row)
                continue
            items.append({"code": split_code(val), "display": val})
        if len(items) != want:
            raise SystemExit(
                f"{key}: có {len(items)} giá trị thật trong {span} dòng, cần {want}. "
                "Template đã đổi — đọc §13.6 của spec trước khi sửa con số này."
            )
        if blanks:
            gaps[key] = blanks
        catalog[key] = items

    if gaps:
        catalog["_gaps"] = gaps

    # Phường/xã: DISPLAY ở cột D, mã tỉnh ở cột C
    px = cells(z, "sheet3", strs)
    wards = []
    row = 2
    while True:
        display = px.get(f"D{row}")
        if not display:
            break
        wards.append(
            {
                "code": split_code(display),
                "display": display,
                "tinh": px.get(f"C{row}", ""),
            }
        )
        row += 1
    if len(wards) != WARD_COUNT:
        raise SystemExit(f"phuong_xa: có {len(wards)} mục, cần {WARD_COUNT}. Template đã đổi.")
    catalog["phuong_xa"] = wards

    # Assert chéo: mỗi named range PX_<mã> phải khớp số phường có tinh = <mã>
    wb = z.read("xl/workbook.xml").decode("utf-8")
    ranges = re.findall(
        r'<definedName name="PX_(\d+)">PHUONG_XA!\$D\$(\d+):\$D\$(\d+)</definedName>', wb
    )
    if not ranges:
        raise SystemExit("Không thấy named range PX_* nào — template đã đổi cấu trúc.")
    for ma, lo, hi in ranges:
        want_n = int(hi) - int(lo) + 1
        got_n = sum(1 for w in wards if w["tinh"] == ma)
        if want_n != got_n:
            raise SystemExit(
                f"PX_{ma}: named range có {want_n} dòng nhưng cột MATT đếm được "
                f"{got_n}. Danh mục không nhất quán, dừng."
            )

    tinh_codes = {t["code"] for t in catalog["tinh_thanh"]}
    orphan = sorted({w["tinh"] for w in wards} - tinh_codes)
    if orphan:
        raise SystemExit(f"Phường thuộc tỉnh không có trong TINH_THANH: {orphan}")

    OUT.write_text(json.dumps(catalog, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Đã ghi {OUT}")
    for key, _, _, span, want in ENUMS:
        note = f"  (named range {span} dòng, {span - want} ô trống)" if span != want else ""
        print(f"  {key}: {want}{note}")
    print(f"  phuong_xa: {len(wards)}")
    print(f"  {len(ranges)} named range PX_* khớp cột MATT")
    if gaps:
        print("\nCẢNH BÁO — template có lỗ trong danh mục:")
        for key, rows in gaps.items():
            print(f"  {key}: thiếu giá trị ở dòng {rows}")
        print("  Khách mang quốc tịch không có trong danh sách sẽ không khai được")
        print("  bằng mã chuẩn. Xem §13.9 của spec.")


if __name__ == "__main__":
    sys.exit(main())
