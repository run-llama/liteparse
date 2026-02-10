from flask import Flask, request, jsonify
from paddleocr import PaddleOCR
from PIL import Image
import io
import numpy as np

app = Flask(__name__)
ocr = None
current_language = None

# Language mapping from ISO codes to PaddleOCR codes
LANGUAGE_MAP = {
    'en': 'en',
    'zh': 'ch',
    'zh-cn': 'ch',
    'zh-hans': 'ch',
    'zh-tw': 'chinese_cht',
    'zh-hant': 'chinese_cht',
    'ja': 'japan',
    'ko': 'korean',
    'fr': 'french',
    'de': 'german',
    'es': 'spanish',
    'pt': 'portuguese',
    'ru': 'russian',
    'ar': 'arabic',
    'hi': 'devanagari',
}

@app.route('/ocr', methods=['POST'])
def ocr_endpoint():
    global ocr, current_language

    # Get language from request
    language = request.form.get('language', 'en').lower()
    paddle_lang = LANGUAGE_MAP.get(language, language)

    # Initialize OCR if needed or language changed
    if ocr is None or current_language != paddle_lang:
        print(f"Initializing PaddleOCR for language: {paddle_lang}")
        ocr = PaddleOCR(
            use_angle_cls=True,
            lang=paddle_lang,
            use_gpu=False,
            show_log=False
        )
        current_language = paddle_lang

    # Read image
    file = request.files.get('file')
    if not file:
        return jsonify({'error': 'No file provided'}), 400

    # Load image
    image_data = file.read()
    image = Image.open(io.BytesIO(image_data))

    # Convert to numpy array (RGB)
    if image.mode != 'RGB':
        image = image.convert('RGB')
    image_array = np.array(image)

    # Run OCR
    # PaddleOCR returns: [[[bbox], (text, confidence)], ...]
    # where bbox is [[x1,y1], [x2,y2], [x3,y3], [x4,y4]]
    results = ocr.ocr(image_array, cls=True)

    # Format results according to LiteParse OCR API spec
    # Convert to: { text, bbox: [x1, y1, x2, y2], confidence }
    formatted = []

    if results and results[0]:
        for line in results[0]:
            # line is [[[x1,y1], [x2,y2], [x3,y3], [x4,y4]], (text, confidence)]
            coords, (text, confidence) = line

            # Convert polygon to axis-aligned bounding box
            xs = [point[0] for point in coords]
            ys = [point[1] for point in coords]
            bbox = [min(xs), min(ys), max(xs), max(ys)]

            formatted.append({
                'text': text,
                'bbox': bbox,
                'confidence': float(confidence)
            })

    return jsonify({
        'results': formatted
    })

@app.route('/health', methods=['GET'])
def health():
    return jsonify({'status': 'healthy'})

if __name__ == '__main__':
    app.run(host='0.0.0.0', port=8829)
