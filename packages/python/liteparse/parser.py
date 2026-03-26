"""LiteParse Python wrapper - wraps the Node.js CLI via subprocess."""

import asyncio
import json
import os
import shutil
import subprocess
import tempfile
import warnings
from pathlib import Path
from typing import Any, List, Literal, Optional, Union, cast

from .types import (
    BatchResult,
    BoundingBox,
    CLINotFoundError,
    ImageFormat,
    OutputFormat,
    ParsedPage,
    ParseError,
    ParseResult,
    ScreenshotBatchResult,
    ScreenshotResult,
    TextItem,
)


def _find_cli(install_if_not_available: bool) -> str:
    """Find the liteparse CLI executable."""
    # Check if liteparse is in PATH
    cli_path = shutil.which("liteparse")
    if cli_path:
        return cli_path

    # Check if npx is available
    npx_path = shutil.which("npx")
    if npx_path:
        return "npx liteparse"

    # Check common node_modules locations
    possible_paths = [
        "./node_modules/.bin/liteparse",
        "../node_modules/.bin/liteparse",
        "../../node_modules/.bin/liteparse",
    ]

    for path in possible_paths:
        if os.path.isfile(path):
            return os.path.abspath(path)

    if install_if_not_available:
        warnings.warn(
            "liteparse CLI could not be find. Running `npm install -g @llamaindex/liteparse` to install it.",
            UserWarning,
        )
        result = subprocess.run(
            ["npm", "install", "-g", "@llamaindex/liteparse"],
        )
        if result.returncode != 0:
            raise subprocess.CalledProcessError(
                result.returncode,
                ["npm", "install", "-g", "@llamaindex/liteparse"],
                result.stderr,
            )
        cli_path = shutil.which("liteparse")
        if cli_path:
            return cli_path

    raise CLINotFoundError(
        "liteparse CLI not found. Please install it with: npm i -g @llamaindex/liteparse"
    )


def _parse_json_result(json_data: dict) -> ParseResult:
    """Parse JSON output from CLI into ParseResult."""
    pages: List[ParsedPage] = []

    for page_data in json_data.get("pages", []):
        # Parse text items
        text_items: List[TextItem] = []
        for item in page_data.get("textItems", []):
            text_items.append(
                TextItem(
                    text=item.get("text", ""),
                    x=item.get("x", 0),
                    y=item.get("y", 0),
                    width=item.get("width", 0),
                    height=item.get("height", 0),
                    fontName=item.get("fontName"),
                    fontSize=item.get("fontSize"),
                )
            )

        # Parse bounding boxes
        bounding_boxes: List[BoundingBox] = []
        for bbox in page_data.get("boundingBoxes", []):
            bounding_boxes.append(
                BoundingBox(
                    x1=bbox.get("x1", 0),
                    y1=bbox.get("y1", 0),
                    x2=bbox.get("x2", 0),
                    y2=bbox.get("y2", 0),
                )
            )

        pages.append(
            ParsedPage(
                pageNum=page_data.get("page", page_data.get("pageNum", 0)),
                width=page_data.get("width", 0),
                height=page_data.get("height", 0),
                text=page_data.get("text", ""),
                textItems=text_items,
                boundingBoxes=bounding_boxes,
            )
        )

    # Build full text from pages (JSON doesn't have top-level text field)
    full_text = "\n\n".join(page.text for page in pages)

    return ParseResult(
        pages=pages,
        text=full_text,
        json=json_data,
    )


def _build_parse_cli_args(
    ocr_enabled: bool,
    ocr_server_url: Optional[str],
    ocr_language: str,
    num_workers: Optional[int],
    max_pages: int,
    target_pages: Optional[str],
    dpi: int,
    precise_bounding_box: bool,
    preserve_very_small_text: bool,
    password: Optional[str],
) -> List[str]:
    """Build CLI arguments for parse command."""
    args: List[str] = ["--format", "json"]

    if not ocr_enabled:
        args.append("--no-ocr")
    elif ocr_server_url:
        args.extend(["--ocr-server-url", ocr_server_url])

    args.extend(["--ocr-language", ocr_language])

    if num_workers is not None:
        args.extend(["--num-workers", str(num_workers)])

    args.extend(["--max-pages", str(max_pages)])

    if target_pages:
        args.extend(["--target-pages", target_pages])

    args.extend(["--dpi", str(dpi)])

    if not precise_bounding_box:
        args.append("--no-precise-bbox")

    if preserve_very_small_text:
        args.append("--preserve-small-text")

    if password:
        args.extend(["--password", password])

    args.append("-q")
    return args


def _build_batch_cli_args(
    output_format: OutputFormat,
    ocr_enabled: bool,
    ocr_server_url: Optional[str],
    ocr_language: str,
    num_workers: Optional[int],
    max_pages: int,
    dpi: int,
    precise_bounding_box: bool,
    recursive: bool,
    extension_filter: Optional[str],
    password: Optional[str],
) -> List[str]:
    """Build CLI arguments for batch-parse command."""
    args: List[str] = ["--format", output_format.value]

    if not ocr_enabled:
        args.append("--no-ocr")
    elif ocr_server_url:
        args.extend(["--ocr-server-url", ocr_server_url])

    args.extend(["--ocr-language", ocr_language])

    if num_workers is not None:
        args.extend(["--num-workers", str(num_workers)])

    args.extend(["--max-pages", str(max_pages)])
    args.extend(["--dpi", str(dpi)])

    if not precise_bounding_box:
        args.append("--no-precise-bbox")

    if recursive:
        args.append("--recursive")

    if extension_filter:
        args.extend(["--extension", extension_filter])

    if password:
        args.extend(["--password", password])

    args.append("-q")
    return args


class LiteParse:
    """
    Python wrapper for the LiteParse document parser.

    This class wraps the LiteParse Node.js CLI, providing a Pythonic interface
    for parsing PDFs and other documents.

    Example:
        >>> from liteparse import LiteParse
        >>> parser = LiteParse()
        >>> result = parser.parse("document.pdf")
        >>> print(result.text)
    """

    def __init__(
        self, cli_path: Optional[str] = None, install_if_not_available: bool = True
    ):
        """
        Initialize LiteParse parser.

        Args:
            cli_path: Custom path to liteparse CLI (auto-detected if not provided)
            install_if_not_available: Install the liteparse CLI from NPM if not available. Defaults to True.
        """
        self._cli_path = cli_path
        self.install_if_not_available = install_if_not_available

    @property
    def cli_path(self) -> str:
        """Get the CLI path, finding it if not already set."""
        if self._cli_path is None:
            self._cli_path = _find_cli(self.install_if_not_available)
        return self._cli_path

    def _prepare_command(
        self,
        subcommand: Literal["parse", "batch-parse", "screenshot"],
        *positional: Any,
        **options: Any,
    ) -> list[str]:
        cmd_parts = self.cli_path.split()
        cmd = cmd_parts + [subcommand, *positional]
        if subcommand == "parse":
            cmd.extend(_build_parse_cli_args(**options))
        elif subcommand == "batch-parse":
            cmd.extend(_build_batch_cli_args(**options))
        return cmd

    @staticmethod
    def _extract_path_and_bytes(
        file_data: Union[str, Path, bytes],
    ) -> tuple[str, Union[bytes, None]]:
        if not isinstance(file_data, bytes):
            file_path = Path(file_data)
            if not file_path.exists():
                raise FileNotFoundError(f"File not found: {file_path}")
            file_path = str(file_path.absolute())
            file_bytes = None
        else:
            file_path = "-"
            file_bytes = file_data
        return file_path, file_bytes

    @staticmethod
    def _extract_batch_params(
        input_dir: Union[str, Path],
        output_dir: Union[str, Path],
        output_format: Union[OutputFormat, str],
    ) -> tuple[Path, Path, OutputFormat]:
        indir = Path(input_dir)
        outdir = Path(output_dir)

        if not indir.exists():
            raise FileNotFoundError(f"Input directory not found: {input_dir}")

        if isinstance(output_format, str):
            output_format = OutputFormat(output_format)

        return indir, outdir, output_format

    @staticmethod
    def _extract_screenshot_params(
        file_path: Union[str, Path],
        image_format: Union[ImageFormat, str],
        output_dir: Union[str, Path, None],
    ) -> tuple[Path, ImageFormat, Path]:
        file_path = Path(file_path)
        if not file_path.exists():
            raise FileNotFoundError(f"File not found: {file_path}")

        if isinstance(image_format, str):
            image_format = ImageFormat(image_format)

        # Use temp dir if output_dir not provided
        if output_dir is None:
            output_dir = Path(tempfile.mkdtemp(prefix="liteparse_screenshots_"))
        else:
            output_dir = Path(output_dir)
            output_dir.mkdir(parents=True, exist_ok=True)
        return file_path, image_format, output_dir

    @staticmethod
    def _get_parse_result(
        returncode: int,
        stdout: bytes,
        stderr: bytes,
    ) -> ParseResult:
        if returncode != 0:
            raise ParseError(
                f"Parsing failed with exit code {returncode}",
                stderr=stderr.decode("utf-8"),
            )
        try:
            json_data = json.loads(stdout.decode("utf-8"))
            return _parse_json_result(json_data)
        except json.JSONDecodeError as e:
            raise ParseError(f"Failed to parse CLI output: {e}")

    @staticmethod
    def _get_screenshot_result(
        returncode: int,
        stderr: str,
        output_dir: Path,
        image_format: ImageFormat,
        load_bytes: bool,
    ) -> ScreenshotBatchResult:
        if returncode != 0:
            raise ParseError(
                f"Screenshot generation failed with exit code {returncode}",
                stderr=stderr,
            )
        screenshots: List[ScreenshotResult] = []
        ext = f".{image_format.value}"

        for img_file in sorted(output_dir.glob(f"*{ext}")):
            # Parse page number from filename (page_N.png)
            filename = img_file.stem
            if filename.startswith("page_"):
                try:
                    page_num = int(filename.replace("page_", ""))
                except ValueError:
                    continue

                # Optionally load bytes
                image_bytes = None
                if load_bytes:
                    image_bytes = img_file.read_bytes()

                screenshots.append(
                    ScreenshotResult(
                        page_num=page_num,
                        image_path=str(img_file),
                        image_bytes=image_bytes,
                    )
                )

        return ScreenshotBatchResult(
            screenshots=screenshots,
            output_dir=str(output_dir),
        )

    def parse(
        self,
        file_data: Union[str, Path, bytes],
        *,
        ocr_enabled: bool = True,
        ocr_server_url: Optional[str] = None,
        ocr_language: str = "en",
        num_workers: Optional[int] = None,
        max_pages: int = 10000,
        target_pages: Optional[str] = None,
        dpi: int = 150,
        precise_bounding_box: bool = True,
        preserve_very_small_text: bool = False,
        password: Optional[str] = None,
        timeout: Optional[float] = None,
    ) -> ParseResult:
        """
        Parse a document file.

        Args:
            file_path: Path to the document file (PDF, DOCX, images, etc.)
            file_bytes: Bytes content of the file
            ocr_enabled: Whether to enable OCR for scanned documents
            ocr_server_url: URL of HTTP OCR server (uses Tesseract if not provided)
            ocr_language: Language code for OCR (e.g., "en", "fr", "de")
            num_workers: Number of pages to OCR in parallel (defaults to CPU cores - 1)
            max_pages: Maximum number of pages to parse
            target_pages: Specific pages to parse (e.g., "1-5,10,15-20")
            dpi: DPI for rendering (affects OCR quality)
            precise_bounding_box: Whether to compute precise bounding boxes
            preserve_very_small_text: Whether to preserve very small text
            password: Password for encrypted/protected documents
            timeout: Timeout in seconds (None for no timeout)

        Returns:
            ParseResult containing the parsed document data

        Raises:
            ParseError: If parsing fails
            FileNotFoundError: If the file doesn't exist
            TimeoutError: If parsing times out
        """

        file_path, file_bytes = self._extract_path_and_bytes(file_data)

        # Build command
        cmd = self._prepare_command(
            "parse",
            file_path,
            ocr_enabled=ocr_enabled,
            ocr_server_url=ocr_server_url,
            ocr_language=ocr_language,
            num_workers=num_workers,
            max_pages=max_pages,
            target_pages=target_pages,
            dpi=dpi,
            precise_bounding_box=precise_bounding_box,
            preserve_very_small_text=preserve_very_small_text,
            password=password,
        )

        try:
            result = subprocess.run(
                cmd,
                capture_output=True,
                timeout=timeout,
                check=False,
                input=file_bytes,
            )

            return self._get_parse_result(
                result.returncode, result.stdout, result.stderr
            )

        except subprocess.TimeoutExpired:
            raise TimeoutError(f"Parsing timed out after {timeout} seconds")

    async def parse_async(
        self,
        file_data: Union[str, Path, bytes],
        *,
        ocr_enabled: bool = True,
        ocr_server_url: Optional[str] = None,
        ocr_language: str = "en",
        num_workers: Optional[int] = None,
        max_pages: int = 10000,
        target_pages: Optional[str] = None,
        dpi: int = 150,
        precise_bounding_box: bool = True,
        preserve_very_small_text: bool = False,
        password: Optional[str] = None,
        timeout: Optional[float] = None,
    ) -> ParseResult:
        """
        Parse a document file (asynchronously).

        Args:
            file_path: Path to the document file (PDF, DOCX, images, etc.)
            file_bytes: Bytes content of the file
            ocr_enabled: Whether to enable OCR for scanned documents
            ocr_server_url: URL of HTTP OCR server (uses Tesseract if not provided)
            ocr_language: Language code for OCR (e.g., "en", "fr", "de")
            num_workers: Number of pages to OCR in parallel (defaults to CPU cores - 1)
            max_pages: Maximum number of pages to parse
            target_pages: Specific pages to parse (e.g., "1-5,10,15-20")
            dpi: DPI for rendering (affects OCR quality)
            precise_bounding_box: Whether to compute precise bounding boxes
            preserve_very_small_text: Whether to preserve very small text
            password: Password for encrypted/protected documents
            timeout: Timeout in seconds (None for no timeout)

        Returns:
            ParseResult containing the parsed document data

        Raises:
            ParseError: If parsing fails
            FileNotFoundError: If the file doesn't exist
            TimeoutError: If parsing times out
        """
        file_path, file_bytes = self._extract_path_and_bytes(file_data)

        # Build command
        cmd = self._prepare_command(
            "parse",
            file_path,
            ocr_enabled=ocr_enabled,
            ocr_server_url=ocr_server_url,
            ocr_language=ocr_language,
            num_workers=num_workers,
            max_pages=max_pages,
            target_pages=target_pages,
            dpi=dpi,
            precise_bounding_box=precise_bounding_box,
            preserve_very_small_text=preserve_very_small_text,
            password=password,
        )

        try:
            process = await asyncio.subprocess.create_subprocess_exec(
                cmd[0],
                *cmd[1:],
                stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
            stdout, stderr = await asyncio.wait_for(
                process.communicate(input=file_bytes), timeout=timeout
            )

            return self._get_parse_result(cast(int, process.returncode), stdout, stderr)

        except TimeoutError:
            raise TimeoutError(f"Parsing timed out after {timeout} seconds")

    def batch_parse(
        self,
        input_dir: Union[str, Path],
        output_dir: Union[str, Path],
        *,
        output_format: Union[OutputFormat, str] = OutputFormat.TEXT,
        ocr_enabled: bool = True,
        ocr_server_url: Optional[str] = None,
        ocr_language: str = "en",
        num_workers: Optional[int] = None,
        max_pages: int = 10000,
        dpi: int = 150,
        precise_bounding_box: bool = True,
        recursive: bool = False,
        extension_filter: Optional[str] = None,
        password: Optional[str] = None,
        timeout: Optional[float] = None,
    ) -> BatchResult:
        """
        Parse multiple documents in batch mode.

        This is more efficient than calling parse() multiple times because
        it reuses the PDF engine across files, avoiding cold-start overhead.

        Args:
            input_dir: Directory containing documents to parse
            output_dir: Directory to write output files
            output_format: Output format ("json" or "text")
            ocr_enabled: Whether to enable OCR for scanned documents
            ocr_server_url: URL of HTTP OCR server (uses Tesseract if not provided)
            ocr_language: Language code for OCR
            num_workers: Number of pages to OCR in parallel (defaults to CPU cores - 1)
            max_pages: Maximum number of pages to parse per file
            dpi: DPI for rendering
            precise_bounding_box: Whether to compute precise bounding boxes
            recursive: Whether to recursively search subdirectories
            extension_filter: Only process files with this extension (e.g., ".pdf")
            password: Password for encrypted/protected documents (applied to all files)
            timeout: Timeout in seconds for the entire batch

        Returns:
            BatchResult with output directory path

        Raises:
            FileNotFoundError: If the input directory doesn't exist
            TimeoutError: If the batch operation times out
        """

        input_dir, output_dir, output_format = self._extract_batch_params(
            input_dir, output_dir, output_format
        )

        # Build command
        cmd = self._prepare_command(
            "batch-parse",
            str(input_dir.absolute()),
            str(output_dir.absolute()),
            output_format=output_format,
            ocr_enabled=ocr_enabled,
            ocr_server_url=ocr_server_url,
            ocr_language=ocr_language,
            num_workers=num_workers,
            max_pages=max_pages,
            dpi=dpi,
            precise_bounding_box=precise_bounding_box,
            recursive=recursive,
            extension_filter=extension_filter,
            password=password,
        )

        try:
            subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=timeout,
                check=False,
            )

            return BatchResult(output_dir=str(output_dir))

        except subprocess.TimeoutExpired:
            raise TimeoutError(f"Batch parsing timed out after {timeout} seconds")

    async def batch_parse_async(
        self,
        input_dir: Union[str, Path],
        output_dir: Union[str, Path],
        *,
        output_format: Union[OutputFormat, str] = OutputFormat.TEXT,
        ocr_enabled: bool = True,
        ocr_server_url: Optional[str] = None,
        ocr_language: str = "en",
        num_workers: Optional[int] = None,
        max_pages: int = 10000,
        dpi: int = 150,
        precise_bounding_box: bool = True,
        recursive: bool = False,
        extension_filter: Optional[str] = None,
        password: Optional[str] = None,
        timeout: Optional[float] = None,
    ) -> BatchResult:
        """
        Parse multiple documents in batch mode (asynchronously).

        This is more efficient than calling parse() multiple times because
        it reuses the PDF engine across files, avoiding cold-start overhead.

        Args:
            input_dir: Directory containing documents to parse
            output_dir: Directory to write output files
            output_format: Output format ("json" or "text")
            ocr_enabled: Whether to enable OCR for scanned documents
            ocr_server_url: URL of HTTP OCR server (uses Tesseract if not provided)
            ocr_language: Language code for OCR
            num_workers: Number of pages to OCR in parallel (defaults to CPU cores - 1)
            max_pages: Maximum number of pages to parse per file
            dpi: DPI for rendering
            precise_bounding_box: Whether to compute precise bounding boxes
            recursive: Whether to recursively search subdirectories
            extension_filter: Only process files with this extension (e.g., ".pdf")
            password: Password for encrypted/protected documents (applied to all files)
            timeout: Timeout in seconds for the entire batch

        Returns:
            BatchResult with output directory path

        Raises:
            FileNotFoundError: If the input directory doesn't exist
            TimeoutError: If the batch operation times out
        """
        input_dir, output_dir, output_format = self._extract_batch_params(
            input_dir, output_dir, output_format
        )

        # Build command
        cmd = self._prepare_command(
            "batch-parse",
            str(input_dir.absolute()),
            str(output_dir.absolute()),
            output_format=output_format,
            ocr_enabled=ocr_enabled,
            ocr_server_url=ocr_server_url,
            ocr_language=ocr_language,
            num_workers=num_workers,
            max_pages=max_pages,
            dpi=dpi,
            precise_bounding_box=precise_bounding_box,
            recursive=recursive,
            extension_filter=extension_filter,
            password=password,
        )

        try:
            process = await asyncio.subprocess.create_subprocess_exec(
                cmd[0],
                *cmd[1:],
            )
            await asyncio.wait_for(process.wait(), timeout=timeout)

            return BatchResult(output_dir=str(output_dir))

        except TimeoutError:
            raise TimeoutError(f"Batch parsing timed out after {timeout} seconds")

    def screenshot(
        self,
        file_path: Union[str, Path],
        output_dir: Optional[Union[str, Path]] = None,
        *,
        target_pages: Optional[str] = None,
        dpi: int = 150,
        image_format: Union[ImageFormat, str] = ImageFormat.PNG,
        password: Optional[str] = None,
        load_bytes: bool = False,
        timeout: Optional[float] = None,
    ) -> ScreenshotBatchResult:
        """
        Generate screenshots of document pages.

        Args:
            file_path: Path to the document file
            output_dir: Directory to save screenshots (uses temp dir if not provided)
            target_pages: Specific pages to screenshot (e.g., "1,3,5" or "1-5")
            dpi: DPI for rendering
            image_format: Image format ("png" or "jpg")
            password: Password for encrypted/protected documents
            load_bytes: If True, load image bytes into ScreenshotResult objects
            timeout: Timeout in seconds

        Returns:
            ScreenshotBatchResult containing paths to generated screenshots

        Raises:
            FileNotFoundError: If the file doesn't exist
            TimeoutError: If the operation times out
        """
        file_path, image_format, output_dir = self._extract_screenshot_params(
            file_path, image_format, output_dir
        )

        # Build command
        cmd = self._prepare_command(
            "screenshot",
            str(file_path.absolute()),
            "-o",
            str(output_dir.absolute()),
            "--format",
            image_format.value,
            "--dpi",
            str(dpi),
            "-q",
        )

        if target_pages:
            cmd.extend(["--target-pages", target_pages])

        if password:
            cmd.extend(["--password", password])

        try:
            result = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=timeout,
                check=False,
            )

            return self._get_screenshot_result(
                result.returncode,
                result.stderr,
                output_dir,
                image_format,
                load_bytes,
            )

        except subprocess.TimeoutExpired:
            raise TimeoutError(
                f"Screenshot generation timed out after {timeout} seconds"
            )

    async def screenshot_async(
        self,
        file_path: Union[str, Path],
        output_dir: Optional[Union[str, Path]] = None,
        *,
        target_pages: Optional[str] = None,
        dpi: int = 150,
        image_format: Union[ImageFormat, str] = ImageFormat.PNG,
        password: Optional[str] = None,
        load_bytes: bool = False,
        timeout: Optional[float] = None,
    ) -> ScreenshotBatchResult:
        """
        Generate screenshots of document pages (asynchronously).

        Args:
            file_path: Path to the document file
            output_dir: Directory to save screenshots (uses temp dir if not provided)
            target_pages: Specific pages to screenshot (e.g., "1,3,5" or "1-5")
            dpi: DPI for rendering
            image_format: Image format ("png" or "jpg")
            password: Password for encrypted/protected documents
            load_bytes: If True, load image bytes into ScreenshotResult objects
            timeout: Timeout in seconds

        Returns:
            ScreenshotBatchResult containing paths to generated screenshots

        Raises:
            FileNotFoundError: If the file doesn't exist
            TimeoutError: If the operation times out
        """
        file_path, image_format, output_dir = self._extract_screenshot_params(
            file_path, image_format, output_dir
        )

        # Build command
        cmd = self._prepare_command(
            "screenshot",
            str(file_path.absolute()),
            "-o",
            str(output_dir.absolute()),
            "--format",
            image_format.value,
            "--dpi",
            str(dpi),
            "-q",
        )

        if target_pages:
            cmd.extend(["--target-pages", target_pages])

        if password:
            cmd.extend(["--password", password])

        try:
            process = await asyncio.subprocess.create_subprocess_exec(cmd[0], *cmd[1:])

            _, stderr = await asyncio.wait_for(process.communicate(), timeout=timeout)

            return self._get_screenshot_result(
                cast(int, process.returncode),
                (stderr or b"").decode("utf-8"),
                output_dir,
                image_format,
                load_bytes,
            )

        except TimeoutError:
            raise TimeoutError(
                f"Screenshot generation timed out after {timeout} seconds"
            )

    def __repr__(self) -> str:
        return f"LiteParse(cli_path={self._cli_path!r})"
