import type {
  DocumentAnnotation,
  FormField,
  ExtractedImage,
  PageComplexityStats,
  ParseResult,
  TextItem,
  VectorGraphics,
  StructureTreeElement,
  XfaPacket,
} from "./lib.js";

function textItemToCliJson(item: TextItem, extractTextMetadata: boolean) {
  return {
    text: item.text,
    x: item.x,
    y: item.y,
    width: item.width,
    height: item.height,
    ...(extractTextMetadata && item.rotation !== undefined
      ? { rotation: item.rotation }
      : {}),
    ...(item.fontName !== undefined ? { font_name: item.fontName } : {}),
    ...(item.fontSize !== undefined ? { font_size: item.fontSize } : {}),
    ...(item.fontHeight !== undefined ? { font_height: item.fontHeight } : {}),
    ...(item.fontAscent !== undefined ? { font_ascent: item.fontAscent } : {}),
    ...(item.fontDescent !== undefined ? { font_descent: item.fontDescent } : {}),
    ...(item.fontWeight !== undefined ? { font_weight: item.fontWeight } : {}),
    ...(item.textWidth !== undefined ? { text_width: item.textWidth } : {}),
    ...(item.fontIsBuggy !== undefined ? { font_is_buggy: item.fontIsBuggy } : {}),
    ...(item.mcid !== undefined ? { mcid: item.mcid } : {}),
    ...(item.fillColor !== undefined ? { fill_color: item.fillColor } : {}),
    ...(item.strokeColor !== undefined ? { stroke_color: item.strokeColor } : {}),
    ...(item.charCodes?.length ? { char_codes: item.charCodes } : {}),
    ...(item.trailingSpaceGenerated
      ? { trailing_space_generated: true }
      : {}),
    // The CLI JSON format reports native text as fully confident, matching the
    // Rust CLI. The `confidence` field on TextItem itself stays undefined for
    // native text so it can discriminate OCR-derived items.
    confidence: item.confidence ?? 1.0,
  };
}

function structureTreeElementToCliJson(element: StructureTreeElement): object {
  return {
    type: element.type,
    ...(element.id !== undefined ? { id: element.id } : {}),
    ...(element.actualText !== undefined ? { actual_text: element.actualText } : {}),
    ...(element.altText !== undefined ? { alt_text: element.altText } : {}),
    ...(element.title !== undefined ? { title: element.title } : {}),
    ...(Object.keys(element.attributes).length ? { attributes: element.attributes } : {}),
    marked_content_ids: element.markedContentIds,
    children: element.children.map(structureTreeElementToCliJson),
    annotations: element.annotations.map(annotationToCliJson),
  };
}

function complexityToCliJson(complexity: PageComplexityStats) {
  return {
    page_number: complexity.pageNumber,
    text_length: complexity.textLength,
    text_coverage: complexity.textCoverage,
    has_substantial_images: complexity.hasSubstantialImages,
    image_block_count: complexity.imageBlockCount,
    image_coverage: complexity.imageCoverage,
    largest_image_coverage: complexity.largestImageCoverage,
    full_page_image: complexity.fullPageImage,
    ...(complexity.uncoveredVectorArea !== undefined
      ? { uncovered_vector_area: complexity.uncoveredVectorArea }
      : {}),
    is_garbled: complexity.isGarbled,
    page_area: complexity.pageArea,
    needs_ocr: complexity.needsOcr,
    reasons: complexity.reasons,
    ...(complexity.layout
      ? {
          layout: {
            column_count: complexity.layout.columnCount,
            ruled_table_count: complexity.layout.ruledTableCount,
            ruled_table_coverage: complexity.layout.ruledTableCoverage,
            text_table_run_count: complexity.layout.textTableRunCount,
            figure_count: complexity.layout.figureCount,
            figure_coverage: complexity.layout.figureCoverage,
            is_complex: complexity.layout.isComplex,
            reasons: complexity.layout.reasons,
          },
        }
      : {}),
  };
}

function vectorGraphicsToCliJson(vectorGraphics: VectorGraphics) {
  return {
    shapes: vectorGraphics.shapes.map((shape) => ({
      bbox: {
        x: shape.bbox.x,
        y: shape.bbox.y,
        width: shape.bbox.width,
        height: shape.bbox.height,
      },
      stroke: shape.stroke,
      ...(shape.strokeColor !== undefined
        ? { stroke_color: shape.strokeColor }
        : {}),
      fill: shape.fill,
      ...(shape.fillColor !== undefined
        ? { fill_color: shape.fillColor }
        : {}),
      has_curve: shape.hasCurve,
    })),
    lines: vectorGraphics.lines.map((line) => ({
      x1: line.x1,
      y1: line.y1,
      x2: line.x2,
      y2: line.y2,
      stroke: line.stroke,
      ...(line.strokeWidth !== undefined
        ? { stroke_width: line.strokeWidth }
        : {}),
      ...(line.strokeColor !== undefined
        ? { stroke_color: line.strokeColor }
        : {}),
      fill: line.fill,
      ...(line.fillColor !== undefined ? { fill_color: line.fillColor } : {}),
    })),
  };
}

function annotationToCliJson(annotation: DocumentAnnotation) {
  return {
    subtype: annotation.subtype,
    ...(annotation.contents !== undefined
      ? { contents: annotation.contents }
      : {}),
    ...(annotation.created !== undefined ? { created: annotation.created } : {}),
    ...(annotation.modified !== undefined
      ? { modified: annotation.modified }
      : {}),
    ...(annotation.title !== undefined ? { title: annotation.title } : {}),
    ...(annotation.rect !== undefined ? { rect: annotation.rect } : {}),
    ...(annotation.quadpointRects.length
      ? {
          quadpoint_rects: annotation.quadpointRects.map((rect) => ({
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
          })),
        }
      : {}),
    ...(annotation.uri !== undefined ? { uri: annotation.uri } : {}),
  };
}

function formFieldToCliJson(field: FormField) {
  return {
    id: field.id,
    type: field.type,
    page: field.page,
    annotation_index: field.annotationIndex,
    widget_index: field.widgetIndex,
    ...(field.objectNumber !== undefined ? { object_number: field.objectNumber } : {}),
    ...(field.name !== undefined ? { name: field.name } : {}),
    ...(field.alternateName !== undefined ? { alternate_name: field.alternateName } : {}),
    ...(field.value !== undefined ? { value: field.value } : {}),
    ...(field.exportValue !== undefined ? { export_value: field.exportValue } : {}),
    field_flags: field.fieldFlags,
    ...(field.controlCount !== undefined ? { control_count: field.controlCount } : {}),
    ...(field.controlIndex !== undefined ? { control_index: field.controlIndex } : {}),
    ...(field.checked !== undefined ? { checked: field.checked } : {}),
    ...(field.rect !== undefined ? { rect: field.rect } : {}),
    ...(field.options.length ? { options: field.options } : {}),
    ...(field.selectedOptions.length ? { selected_options: field.selectedOptions } : {}),
  };
}

function xfaPacketToCliJson(packet: XfaPacket) {
  return {
    index: packet.index,
    ...(packet.name !== undefined ? { name: packet.name } : {}),
    content_length: packet.contentLength,
    ...(packet.content !== undefined ? { content: packet.content } : {}),
  };
}

function imageToCliJson(image: ExtractedImage) {
  return {
    id: image.id,
    name: image.name,
    ...(image.path !== undefined ? { path: image.path } : {}),
    page: image.page,
    bbox: {
      x: image.bbox.x,
      y: image.bbox.y,
      width: image.bbox.width,
      height: image.bbox.height,
    },
    width: image.width,
    height: image.height,
    rotation: image.rotation,
    format: image.format,
    ...(image.duplicateOf !== undefined
      ? { duplicate_of: image.duplicateOf }
      : {}),
  };
}

/** Project the camelCase Node API result into the Rust CLI JSON schema. */
export function parseResultToCliJson(
  result: ParseResult,
  options: { extractTextMetadata?: boolean } = {},
) {
  const extractTextMetadata = options.extractTextMetadata ?? false;
  return {
    outline: result.outline.map((entry) => ({
      level: entry.level,
      title: entry.title,
      page_index: entry.pageIndex,
      y_pdf: entry.yPdf ?? null,
    })),
    pages: result.pages.map((page) => ({
      page: page.pageNum,
      width: page.width,
      height: page.height,
      ...(page.contentBounds
        ? {
            content_bounds: {
              x: page.contentBounds.x,
              y: page.contentBounds.y,
              width: page.contentBounds.width,
              height: page.contentBounds.height,
            },
          }
        : {}),
      text: page.text,
      text_items: page.textItems.map((item) =>
        textItemToCliJson(item, extractTextMetadata),
      ),
      ...(page.complexity
        ? { complexity: complexityToCliJson(page.complexity) }
        : {}),
      ...(page.vectorGraphics
        ? { vector_graphics: vectorGraphicsToCliJson(page.vectorGraphics) }
        : {}),
      ...(page.annotations !== undefined
        ? { annotations: page.annotations.map(annotationToCliJson) }
        : {}),
      ...(page.formFields !== undefined
        ? { form_fields: page.formFields.map(formFieldToCliJson) }
        : {}),
      ...(page.structureTree !== undefined
        ? {
            structure_tree: {
              roots: page.structureTree.roots.map(structureTreeElementToCliJson),
            },
          }
        : {}),
    })),
    ...(result.images.length
      ? { images: result.images.map(imageToCliJson) }
      : {}),
    ...(result.imageErrorCount
      ? { image_error_count: result.imageErrorCount }
      : {}),
    ...(result.formType !== undefined ? { form_type: result.formType } : {}),
    ...(result.xfaPackets !== undefined
      ? { xfa_packets: result.xfaPackets.map(xfaPacketToCliJson) }
      : {}),
  };
}
