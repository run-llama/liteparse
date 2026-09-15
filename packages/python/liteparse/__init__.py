"""LiteParse - fast local document parsing."""

# Re-exports are resolved lazily (PEP 562). `lit` runs cli.py, which imports
# liteparse._liteparse and so executes this module; eagerly pulling in .parser
# and .types dragged asyncio and importlib.metadata into every CLI invocation
# for ~45ms of startup that a parse never uses.
_LAZY = {
    "LiteParse": ".parser",
    "search_items": ".parser",
    "AnnotationRect": ".types",
    "DocumentAnnotation": ".types",
    "LayoutBlock": ".types",
    "LayoutCell": ".types",
    "FormField": ".types",
    "StructureTree": ".types",
    "StructureTreeElement": ".types",
    "ExtractedImage": ".types",
    "ImageRect": ".types",
    "LayoutComplexityStats": ".types",
    "LiteParseConfig": ".types",
    "PageComplexityStats": ".types",
    "PageError": ".types",
    "ParseResult": ".types",
    "ParseBatch": ".types",
    "DocumentMetadata": ".types",
    "XfaPacket": ".types",
    "ParsedPage": ".types",
    "TextItem": ".types",
    "WordBox": ".types",
    "ScreenshotRect": ".types",
    "ScreenshotResult": ".types",
    "ParseError": ".types",
    "ParseTimeoutError": ".types",
    "VectorGraphics": ".types",
    "VectorLine": ".types",
    "VectorShape": ".types",
}


def __getattr__(name):
    if name == "__version__":
        from importlib.metadata import PackageNotFoundError, version

        try:
            value = version("liteparse")
        except PackageNotFoundError:  # source tree without installed dist metadata
            value = "0.0.0+unknown"
    else:
        module = _LAZY.get(name)
        if module is None:
            raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
        from importlib import import_module

        value = getattr(import_module(module, __name__), name)
    globals()[name] = value  # cache: __getattr__ only fires on the first miss
    return value


def __dir__():
    return sorted(set(globals()) | set(_LAZY) | {"__version__"})


__all__ = [
    "LiteParse",
    "AnnotationRect",
    "DocumentAnnotation",
    "LayoutBlock",
    "LayoutCell",
    "FormField",
    "StructureTree",
    "StructureTreeElement",
    "LiteParseConfig",
    "ParseResult",
    "PageError",
    "ParseBatch",
    "DocumentMetadata",
    "XfaPacket",
    "ParsedPage",
    "TextItem",
    "WordBox",
    "ScreenshotRect",
    "ScreenshotResult",
    "PageComplexityStats",
    "LayoutComplexityStats",
    "ExtractedImage",
    "ImageRect",
    "ParseError",
    "ParseTimeoutError",
    "search_items",
    "VectorGraphics",
    "VectorLine",
    "VectorShape",
]
