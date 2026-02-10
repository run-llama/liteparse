from flask import Flask, request, jsonify
import easyocr
import numpy as np
from PIL import Image
import io

app = Flask(__name__)
reader = None
current_language = None

@app.route('/ocr', methods=['POST'])
def ocr():
    global reader, current_language

    # Get language from request
    language = request.form.get('language', 'en')

    # Initialize reader if needed or language changed
    if reader is None or current_language != language:
        print(f"Initializing EasyOCR reader for language: {language}")
        reader = easyocr.Reader([language], gpu=False)
        current_language = language

    # Read image
    file = request.files.get('file')
    if not file:
        return jsonify({'error': 'No file provided'}), 400

    # Load image
    image_data = file.read()
    image = Image.open(io.BytesIO(image_data))

    # Convert to numpy array
    image_array = np.array(image)

    # Run OCR
    results = reader.readtext(image_array)

    # Format results according to LiteParse OCR API spec
    # Convert from EasyOCR format: [[[x1,y1], [x2,y2], [x3,y3], [x4,y4]], text, confidence]
    # To standard format: { text, bbox: [x1, y1, x2, y2], confidence }
    formatted = []
    for coords, text, confidence in results:
        # Convert polygon to axis-aligned bounding box
        # coords is [[x1,y1], [x2,y2], [x3,y3], [x4,y4]]
        if isinstance(coords, np.ndarray):
            coords = coords.tolist()

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
    app.run(host='0.0.0.0', port=8828)
