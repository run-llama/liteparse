FROM python:3.10-slim

# Install system dependencies
RUN apt-get update && apt-get install -y \
    libgomp1 \
    libglib2.0-0 \
    libsm6 \
    libxext6 \
    libxrender-dev \
    libgl1 \
    && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /app

# Install Python dependencies
RUN pip install --no-cache-dir \
    flask==3.0.0 \
    paddlepaddle==3.2.2 \
    paddleocr==3.4.0 \
    pillow \
    numpy

# Copy server code
COPY server.py .

# Expose port
EXPOSE 8829

# Run server
CMD ["python", "server.py"]
