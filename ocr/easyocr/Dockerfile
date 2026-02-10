FROM python:3.9-slim

# Install dependencies
RUN apt-get update && apt-get install -y \
    libglib2.0-0 \
    libsm6 \
    libxext6 \
    libxrender-dev \
    libgomp1 \
    && rm -rf /var/lib/apt/lists/*

# Install Python packages
RUN pip install --no-cache-dir easyocr flask pillow numpy

# Copy server code
COPY server.py /app/server.py

WORKDIR /app

# Expose port
EXPOSE 8828

# Run server
CMD ["python", "server.py"]
