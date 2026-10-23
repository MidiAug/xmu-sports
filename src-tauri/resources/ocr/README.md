# OCR models for the Windows installer (not committed).
#
# Layout expected by tauri bundle + runtime seed:
#   rapidocr/ch_PP-OCRv5_det_mobile.onnx
#   rapidocr/ch_PP-OCRv5_rec_mobile.onnx
#   rapidocr/ppocrv5_dict.txt
#   captcha/common.onnx
#
# Populate before `npm run tauri:build`:
#   powershell -File scripts/ensure_ocr_bundle.ps1
