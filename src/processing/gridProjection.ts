import { strToSubscriptString, strToPostScript } from './textUtils.js';
import { buildBbox } from './bbox.js';
import { cleanRawText } from './cleanText.js';
import { ProjectionTextBox, Coordinates, LiteParseConfig, ParsedPage } from '../core/types.js';
import { PageData } from '../engines/pdf/interface.js';

import { getVerticalAndHorizontalLinesFromPagePaths, VerticalLine } from './pathUtils.js';
import { applyMarkupTags } from './markupUtils.js';

// Minimum spaces between unsnapped bboxes (likely justified text
const FLOATING_SPACES = 2;
// Minimum spaces between snapped columns
const COLUMN_SPACES = 4;


type Snap = {
    bbox: ProjectionTextBox;
    lineIndex: number;
    boxIndex: number;
}

type Anchor = {
    [key: number]: ProjectionTextBox[];
}

type Anchors = {
    anchorLeft: Anchor;
    anchorRight: Anchor;
    anchorCenter: Anchor;
};

type ForwardAnchor = { [key: string]: number };

type PrevAnchors = {
    forwardAnchorLeft: ForwardAnchor;
    forwardAnchorCenter: ForwardAnchor;
    forwardAnchorRight: ForwardAnchor;
}

type PageForwardAnchors = {
    left: ForwardAnchor,
    right: ForwardAnchor,
    center: ForwardAnchor,
    floating: ForwardAnchor,
}

type SnapMaps = {
    left: number[],
    right: number[],
    center: number[],
    floating: number[],
}

type LineRange = {
    start: number,
    end: number,
}

type TextBoxSize = {
    width: number,
    height: number,
}

function roundAnchor(anchor: number): number {
    // group anchor x-coord by nearest 1/4 unit
    return Math.round(anchor * 4) / 4;
}

// 2pt @ PDF 72 DPI -> 8px @ 300DPI
const SMALL_FONT_SIZE_THRESHOLD = 2;

function isSmallTextLine(line: ProjectionTextBox[]): boolean {
    // check for line where >50% of the text is very small
    const smallText = line.filter(item => item.h < SMALL_FONT_SIZE_THRESHOLD);
    if (smallText.length / line.length > .5) {
        return true;
    }
    return false;
}

function filterUnprojectableText(config: LiteParseConfig, line: ProjectionTextBox[]):  ProjectionTextBox[] {
    // extract all text (OSS always uses fast mode)
    if (line.length === 0) {
        return line;
    }

    let filteredLine = line;
    if (!config.preserveVerySmallText && isSmallTextLine(line)) {
        // remove very small text lines
        filteredLine = filteredLine.filter(item => item.h >= SMALL_FONT_SIZE_THRESHOLD);
    }
    return filteredLine;
}

function canSnapLine(config: LiteParseConfig, line: ProjectionTextBox[]): boolean {
    // force lines that will likely break projection to be unsnapped floating text
    // currently this includes:
    //   - lines of entirely small text
    //
    // NOTE: this assumes undesirable text has already been filtered before projection
    // (i.e. parse mode based removal of text should be done before this in filterUnprojectableText())
    if (line.length === 0) {
        return true;
    }

    if (!config.preserveVerySmallText && isSmallTextLine(line)) {
        return false;
    }
    return true;
}

function fixSparseBlocks(blocks: LineRange[], rawLines: string[]) {
    // compress whitespace in blocks containing very sparse lines (>70% whitespace)
    const regexp = new RegExp(`\\s{${COLUMN_SPACES},}`, "g");
    for (const block of blocks) {
        let total = 0;
        let whitespace = 0;
        for (let i = block.start; i < block.end; ++i) {
            if (!rawLines[i]) {
                continue;
            }
            rawLines[i] = rawLines[i].trimEnd();
            const line = rawLines[i];
            if (line.length === 0) {
                continue;
            }
            total += line.length;
            whitespace += line.match(/\s/g)?.length || 0
        }
        if (total >= 500 && whitespace / total > 0.8) {
            for (let i = block.start; i < block.end; ++i) {
                const line = rawLines[i];
                if (!line || line.length === 0) {
                    continue;
                }
                rawLines[i] = line.replace(regexp, " ".repeat(FLOATING_SPACES))
            }
        }
    }
}

function extractAnchorsPointsFromLines(lines: ProjectionTextBox[][], page: PageData, verticalLines: VerticalLine[]): Anchors {
    const pageHeight = page.height;

    let anchorLeft: Anchor = {};
    let anchorRight: Anchor = {};
    let anchorCenter: Anchor = {};

    // Add anchors from vertical lines to ensure horizontal gaps
    // This creates left and right anchors at vertical line positions
    for (let vLine of verticalLines) {
        const leftAnchor = roundAnchor(vLine.x);
        const rightAnchor = roundAnchor(vLine.x + vLine.width);
        
        if (!anchorLeft[leftAnchor]) {
            anchorLeft[leftAnchor] = [];
        }
        if (!anchorRight[rightAnchor]) {
            anchorRight[rightAnchor] = [];
        }
        
        // Create dummy bboxes spanning the vertical line height to establish the anchor
        const dummyBboxLeft = {
            x: vLine.x,
            y: vLine.y1,
            w: 0,
            h: vLine.height,
            str: '',
            strLength: 0
        } as ProjectionTextBox;
        
        const dummyBboxRight = {
            x: vLine.x + vLine.width,
            y: vLine.y1,
            w: 0,
            h: vLine.height,
            str: '',
            strLength: 0
        } as ProjectionTextBox;
        
        anchorLeft[leftAnchor].push(dummyBboxLeft);
        anchorRight[rightAnchor].push(dummyBboxRight);
    }

    for (let line of lines) {
        for (let bbox of line) {
            let anchor = roundAnchor(bbox.x);
            if (!anchorLeft[anchor]) {
                anchorLeft[anchor] = [];
            }
            anchorLeft[anchor].push(bbox);

            anchor = roundAnchor(bbox.x + bbox.w);
            if (!anchorRight[anchor]) {
                anchorRight[anchor] = [];
            }
            anchorRight[anchor].push(bbox);

            let center = Math.round(bbox.x + bbox.w / 2);
            if (!anchorCenter[center]) {
                anchorCenter[center] = [];
            }
            anchorCenter[center].push(bbox);
        }
    }

    function deltaMin(collection: Anchor, delta: number) {
        for (let anchor in collection) {
            let maxDelta = pageHeight * delta;
            for (let i = 0; i < collection[anchor].length; i++) {
                let shouldKeep = false;
                if (i > 0) {
                    if (collection[anchor][i].y - collection[anchor][i - 1].y < maxDelta) {
                        shouldKeep = true;
                    }
                }
                if (i < collection[anchor].length - 1) {
                    if (collection[anchor][i + 1].y - collection[anchor][i].y < maxDelta) {
                        shouldKeep = true;
                    }
                }

                if (!shouldKeep) {
                    collection[anchor].splice(i--, 1);
                }
            }
        }
    }

    deltaMin(anchorRight, 0.1);
    deltaMin(anchorLeft, 0.2);
    deltaMin(anchorCenter, 0.05);

    function intercept(collection: Anchor) {
        for (let anchor in collection) {
            let shouldKeep = false;
            for (let i = 0; i < collection[anchor].length; i++) {
                if (i > 0) {
                    let intercept = false;
                    // check intercept
                    let a1 = collection[anchor][i - 1];
                    let a2 = collection[anchor][i];
                    for (let line of lines) {
                        if (line.length > 0 && line[0].y > a1.y && line[0].y < a2.y) {
                            for (let item of line) {
                                if (item.x < parseInt(anchor) && item.x + item.w > parseInt(anchor)) {
                                    intercept = true;
                                    break;
                                }
                            }
                            if (intercept) {
                                break;
                            }
                        }
                    }
                    if (!intercept) {
                        shouldKeep = true;
                        break;
                    }
                }
            }
            if (!shouldKeep) {
                delete collection[anchor];
            }
        }
    }

    intercept(anchorRight);
    intercept(anchorLeft);
    intercept(anchorCenter);

    function group(collection: Anchor) {
        for (let anchor in collection) {
            const anchorNum = parseFloat(anchor);
            // merge right
            if (collection[anchorNum + 1] && collection[anchorNum + 1].length > collection[anchorNum].length) {
                collection[anchorNum + 1].push(...collection[anchorNum]);
                delete collection[anchorNum];
            }
            // merge left
            else if (collection[anchorNum - 1] && collection[anchorNum - 1].length > collection[anchorNum].length) {
                collection[anchorNum - 1].push(...collection[anchorNum]);
                delete collection[anchorNum];
            }
        }
    }

    group(anchorLeft);
    group(anchorRight);
    group(anchorCenter);


    function anyAnchorSurvived(bbox: ProjectionTextBox) {
        return roundAnchor(bbox.x) in anchorLeft ||
        roundAnchor(bbox.x + bbox.w) in anchorRight ||
        Math.round(bbox.x + bbox.w / 2) in anchorCenter;
    }

    // Try seeing if a floating bbox would align well with a surviving anchor on a line immediately above or below it
    function tryAlignFloating(collection: Anchor, ANCHOR_MARGIN: number, refXFromBbox: (bbox: ProjectionTextBox) => number, anchorValFromBbox: (bbox: ProjectionTextBox) => number) {
        for (let lineIndex = 0; lineIndex < lines.length; lineIndex++) {
            const line = lines[lineIndex];
            for (let bbox of line) {
                // Only consider floating bboxes
                if (anyAnchorSurvived(bbox)) {
                    continue;
                }
                // Check the lines before and after
                let candidateLines: ProjectionTextBox[][] = [];
                if (lineIndex > 0) {
                    candidateLines.push(lines[lineIndex-1]);
                }
                if (lineIndex < lines.length-1) {
                    candidateLines.push(lines[lineIndex+1]);
                }
                
                // Check candidate lines for:
                    // Possible alignment
                    // Being within the margin
                    // Being the closest of the candidates
                let candidateAnchor: string = "";
                let prevDiff = ANCHOR_MARGIN + 1;
                for (let candLine of candidateLines) {
                    for (let candBBox of candLine) {
                        const candAnchorVal = anchorValFromBbox(candBBox);
                        if (!(candAnchorVal in collection)) {
                            continue;
                        }
                        const xDiff = Math.abs(candAnchorVal - refXFromBbox(bbox));
                        if (xDiff <= ANCHOR_MARGIN && xDiff < prevDiff) {
                            candidateAnchor = candAnchorVal.toString();
                            prevDiff = xDiff;
                        }
                    }
                }
                
                // No candidate found
                if (candidateAnchor.length == 0) {
                    continue;
                }
    
                // Candidate found - update the anchor's bbox list
                collection[parseFloat(candidateAnchor)].push(bbox);
            }
        }
    }

    // Try to left-align floating bboxes
    tryAlignFloating(anchorLeft, 2, (bbox) => bbox.x, (bbox) => roundAnchor(bbox.x));

    // Sort the anchors' lists of bboxes by y-value
    function sortAnchor(collection: Anchor) {
        for (let anchor in collection) {
            collection[anchor].sort((a, b) => a.y - b.y)
        }
    }

    sortAnchor(anchorLeft);
    sortAnchor(anchorRight);
    sortAnchor(anchorCenter);


    // deduplicate
    let duplicates = [];
    for (let anchor in anchorLeft) {
        for (let item of anchorLeft[anchor]) {
            item.snap = 'left';
            item.leftAnchor = anchor;
        }
    }

    for (let anchor in anchorRight) {
        for (let item of anchorRight[anchor]) {
            if (item.snap) {
                item.isDup = true;
                duplicates.push(item);
            }
            item.snap = 'right';
            item.rightAnchor = anchor;
        }
    }

    for (let anchor in anchorCenter) {
        for (let item of anchorCenter[anchor]) {
            if (item.snap && !item.isDup) {
                item.isDup = true;
                duplicates.push(item);
            }
            item.snap = 'center';
            item.centerAnchor = anchor;
        }
    }

    function anchorCounts(item: ProjectionTextBox): number[] {
        let leftCount = 0;
        if (item.leftAnchor) {
            const key = parseFloat(item.leftAnchor);
            leftCount = anchorLeft[key] ? anchorLeft[key].length : 0;
        }
        let rightCount = 0;
        if (item.rightAnchor) {
            const key = parseFloat(item.rightAnchor);
            rightCount = anchorRight[key] ? anchorRight[key].length : 0;
        }
        let centerCount = 0;
        if (item.centerAnchor) {
            const key = parseFloat(item.centerAnchor);
            centerCount = anchorCenter[key] ? anchorCenter[key].length : 0;
        }
        return [leftCount, rightCount, centerCount];
    }

    // find all left aligned blocks, all right aligned blocks, all centered blocks, in that order
    // we cannot check all 3 at once since we may end up double counting potential anchor matches
    // (i.e. we need to exclude block that we know are left/right aligned before counting possible
    // matching centered blocks)

    // find all lefts
    let hasChanged = true;
    while (hasChanged && duplicates.length > 0) {
        hasChanged = false;
        for (let i = duplicates.length - 1; i >= 0; --i) {
            const item = duplicates[i];
            const [leftCount, rightCount, centerCount] = anchorCounts(item);
            if (leftCount >= rightCount && leftCount >= centerCount) {
                item.snap = "left";
                if (item.rightAnchor) {
                    const key = parseFloat(item.rightAnchor);
                    if (anchorRight[key]) {
                        anchorRight[key].splice(anchorRight[key].indexOf(item), 1);
                        hasChanged = true;
                    }
                }
                if (item.centerAnchor) {
                    const key = parseFloat(item.centerAnchor);
                    if (anchorCenter[key]) {
                        anchorCenter[key].splice(anchorCenter[key].indexOf(item), 1);
                        hasChanged = true;
                    }
                }
                duplicates.splice(i, 1);
            }
        }
    }

    // find all rights
    hasChanged = true;
    while (hasChanged && duplicates.length > 0) {
        hasChanged = false;
        for (let i = duplicates.length - 1; i >= 0; --i) {
            const item = duplicates[i];
            const [leftCount, rightCount, centerCount] = anchorCounts(item);
            if (rightCount >= leftCount && rightCount >= centerCount) {
                item.snap = "right";
                if (item.leftAnchor) {
                    const key = parseFloat(item.leftAnchor);
                    if (anchorLeft[key]) {
                        anchorLeft[key].splice(anchorLeft[key].indexOf(item), 1);
                        hasChanged = true;
                    }
                }
                if (item.centerAnchor) {
                    const key = parseFloat(item.centerAnchor);
                    if (anchorCenter[key]) {
                        anchorCenter[key].splice(anchorCenter[key].indexOf(item), 1);
                        hasChanged = true;
                    }
                }
                duplicates.splice(i, 1);
            }
        }
    }

    // remaining duplicates are centered
    for (const item of duplicates) {
        item.snap = "center";
        if (item.leftAnchor) {
            const key = parseFloat(item.leftAnchor);
            if (anchorLeft[key]) {
                anchorLeft[key].splice(anchorLeft[key].indexOf(item), 1);
            }
        }
        if (item.rightAnchor) {
            const key = parseFloat(item.rightAnchor);
            if (anchorRight[key]) {
                anchorRight[key].splice(anchorRight[key].indexOf(item), 1);
            }
        }
    }

    // filter anchors
    // delete singleton
    for (let anchor in anchorLeft) {
        if (anchorLeft[anchor].length < 2) {
            if (anchorLeft[anchor].length) {
                delete anchorLeft[anchor][0].snap;
            }
            delete anchorLeft[anchor];
        }
    }

    for (let anchor in anchorRight) {
        if (anchorRight[anchor].length < 2) {
            if (anchorRight[anchor].length) {
                delete anchorRight[anchor][0].snap;
            }
            delete anchorRight[anchor];
        }
    }

    for (let anchor in anchorCenter) {
        if (anchorCenter[anchor].length < 2) {
            if (anchorCenter[anchor].length) {
                delete anchorCenter[anchor][0].snap;
            }
            delete anchorCenter[anchor];
        }
    }

    return {
        anchorLeft,
        anchorRight,
        anchorCenter
    }
}


function handleRotationReadingOrder(textBbox: ProjectionTextBox[], pageHeight: number) {
    // if no bbox is rotated (.r is set), return
    if (!textBbox.find(b => b.r != 0)) {
        return;
    }

    textBbox.sort((a, b) => {
        return a.x - b.x;
    });

    // Horizontal Axis
    let bboxGroup: ProjectionTextBox[][] = [];
    let currentGroup: ProjectionTextBox[] = [];
    let previousBbox: ProjectionTextBox | null = null;
    for (let bbox of textBbox) {
        if (previousBbox) {
            if (bbox.r == previousBbox.r) {
                currentGroup.push(bbox);
            }
            else {
                // sort currentGroup by bbox.y
                currentGroup.sort((a, b) => a.y - b.y);
                bboxGroup.push(currentGroup);
                currentGroup = [bbox];
            }
        }
        else {
            currentGroup.push(bbox);
        }
        previousBbox = bbox;
    }
    if (currentGroup.length) {
        currentGroup.sort((a, b) => a.y - b.y);
        bboxGroup.push(currentGroup);
    }

    // NOTE/ WARNING: height and width of bbox are NOT rotated beforehand!
    for (let [index, group] of bboxGroup.entries()) {
        if (group[0].r == 90 || group[0].r == 270) {
            // check in all bbox if they are not group[0].r and have overlap in x axis
            let globalOverlap = false;
            for (let bbox of textBbox) {
                if (bbox.r != group[0].r) {
                    let overlap = group.find(b => b.x >= bbox.x && b.x <= bbox.x + bbox.w && bbox.r != b.r);
                    if (overlap) {
                        globalOverlap = true;
                    }
                }
            }

            if (globalOverlap) {
                // rotate bbox to be horizontal
                for (let bbox of group) {
                    if (bbox.d) {
                        bbox.y += bbox.d;
                        bbox.d = 0;
                    }
                    bbox.r = 0;
                    bbox.rotated = true;
                }
            }
            else if (!globalOverlap) {
                // insert the bbox group in the Y axis after previous group and before next group.
                // move Next group by current group height (width as not rotated yet).
                let groupMaxX = Math.max(...group.map(v => v.x + v.w));

                let deltaY = 0;
                if (index != 0) {
                    let previousGroup = bboxGroup[index - 1];
                    let previousGroupMaxY = Math.max(...previousGroup.map(v => v.y + v.h));
                    // pageHeight is radical but garantie no issue of allignement
                    deltaY = previousGroupMaxY + pageHeight;
                }
                // clockwize rotation
                if (group[0].r == 90) {
                    for (let bbox of group) {
                        // w not h here as bbox w/h are not flipped
                        let x = Math.round(bbox.ry ?? bbox.y);
                        let y = (bbox.rx ?? bbox.x) + deltaY;
                        bbox.x = Math.round(x);
                        bbox.y = y;
                        bbox.r = 0;
                        bbox.rotated = true;
                    }
                }
                // counter clockwize rotation
                if (group[0].r == 270) {
                    for (let bbox of group) {
                        let x = Math.round(bbox.ry ?? bbox.y);
                        let y = (bbox.rx ?? bbox.x) + deltaY;
                        bbox.x = Math.round(x);
                        bbox.y = y;
                        bbox.r = 0;
                        bbox.rotated = true;
                    }
                }

                // pageHeight is radical but garantie no issue of allignement
                let globalDelta = deltaY + groupMaxX + pageHeight;

                for (let [otherGroupIndex, other] of bboxGroup.entries()) {
                    if (otherGroupIndex <= index) {
                        continue;
                    }
                    for (let bbox of other) {
                        if (bbox.r == 90 || bbox.r == 270) {
                            bbox.d = (bbox.d ? bbox.d : 0) + globalDelta;
                            continue;
                        }
                        bbox.y += globalDelta;
                    }
                }
            }

        }
    }

    textBbox.sort((a, b) => {
        return a.y - b.y;
    });


    // Vertical Axis
    bboxGroup = [];
    currentGroup = [];
    previousBbox = null as ProjectionTextBox | null;
    for (let bbox of textBbox) {
        if (previousBbox) {
            if (bbox.r == previousBbox.r) {
                currentGroup.push(bbox);
            }
            else {
                // sort currentGroup by bbox.y
                currentGroup.sort((a, b) => a.x - b.x);
                bboxGroup.push(currentGroup);
                currentGroup = [bbox];
            }
        }
        else {
            currentGroup.push(bbox);
        }
        previousBbox = bbox;
    }
    if (currentGroup.length) {
        currentGroup.sort((a, b) => a.x - b.x);
        bboxGroup.push(currentGroup);
    }


    for (const group of bboxGroup) {
        if (group[0].r == 180) {
            // Switch upside down
            for (let bbox of group) {
                bbox.x = Math.round(bbox.ry ?? bbox.y);
                bbox.y = bbox.rx ?? bbox.x;
                bbox.r = 0;
                bbox.rotated = true;
            }
        }
    }
}

export function bboxToLine(textBbox: ProjectionTextBox[], medianWidth: number, medianHeight: number): ProjectionTextBox[][] {
    // sort lines on  first y axis then x axis (top - left)
    textBbox.sort((a, b) => {
        if (a.y == b.y) {
            return a.x - b.x;
        }
        return a.y - b.y;
    });

    function canMergeMarkup(previousBbox: ProjectionTextBox, bbox: ProjectionTextBox): boolean {
        if (!previousBbox.markup && !bbox.markup) {
            return true;
        }
        if (previousBbox.markup && bbox.markup
            && previousBbox.markup.highlight === bbox.markup.highlight
            && previousBbox.markup.underline === bbox.markup.underline
            && previousBbox.markup.squiggly === bbox.markup.squiggly
            && previousBbox.markup.strikeout === bbox.markup.strikeout
        ) {
            return true;
        }
        return false;
    }

    function canMerge(previousBbox: ProjectionTextBox, bbox: ProjectionTextBox): boolean {
        if (bbox.y == previousBbox.y && bbox.h == previousBbox.h) {
            const xDelta = bbox.x - previousBbox.x - previousBbox.w;
            if (((xDelta < 0 && xDelta > -.5) || (xDelta >= 0 && xDelta < .1)) && canMergeMarkup(previousBbox, bbox)) {
                return true;
            }
        }
        return false;
    }

    function mergePageBbox(a: ProjectionTextBox, b: ProjectionTextBox): Coordinates {
        const aBbox = a.pageBbox || {x: a.x, y: a.y, w: a.w, h: a.h};
        const bBbox = b.pageBbox || {x: b.x, y: b.y, w: b.w, h: b.h};
        const left = Math.min(aBbox.x, bBbox.x);
        const top = Math.min(aBbox.y, bBbox.y);
        const right = Math.max(aBbox.x + aBbox.w, bBbox.x + bBbox.w);
        const bottom = Math.max(aBbox.y + aBbox.h, bBbox.y + bBbox.h);
        return {x: left, y: top, w: right - left, h: bottom - top};
    }

    // merge Continuous bbox
    for (let i = 1; i < textBbox.length; i++) {
        let bbox = textBbox[i];
        let previousBbox = textBbox[i - 1];
        if (canMerge(previousBbox, bbox)) {
            previousBbox.w = bbox.x + bbox.w - previousBbox.x;
            previousBbox.str += bbox.str;
            previousBbox.strLength += bbox.strLength;
            previousBbox.pageBbox = mergePageBbox(previousBbox, bbox);
            textBbox.splice(i, 1);
            i--;
        }
    }

    // try to find the bounding box that align as line and group them by line
    let lines: ProjectionTextBox[][] = [];
    let currentLine: ProjectionTextBox[] = [];
    let previousBbox = null;

    for (let bbox of textBbox) {
        if (!previousBbox) {
            currentLine.push(bbox);
        }
        // This is where we define how line are build. to be improved
        else {
            let lineMinY = Math.min(...currentLine.map(v => v.y));
            let lineMaxY = Math.max(...currentLine.map(v => v.y + v.h));

            let lineCollide = false;
            for (let currentLineItemBbox of currentLine) {

                let overlapLenght = Math.min(currentLineItemBbox.x + currentLineItemBbox.w, bbox.x + bbox.w) - Math.max(currentLineItemBbox.x, bbox.x);
                if (overlapLenght > medianWidth / 3) {
                    lineCollide = true;
                    break;
                }
            }
            if (!lineCollide
                && (
                    (bbox.y + bbox.h * .5 >= lineMinY && bbox.y + bbox.h * .5 <= lineMaxY)
                    ||
                    (bbox.y >= lineMinY && bbox.y <= lineMaxY)
                )
            ) {
                currentLine.push(bbox);
            }
            else {
                if (currentLine.length) {
                    lines.push(currentLine);
                }
                currentLine = [bbox];
            }
        }
        previousBbox = bbox;
    }

    if (currentLine.length) {
        lines.push(currentLine);
    }

    // sort each line by x
    for (let line of lines) {
        line.sort((a, b) => a.x - b.x);
    }

    // sort lines by y
    lines.sort((a, b) => a[0].y - b[0].y);
    // merge 'words'
    let mergeThreshold = 1;
    for (let line of lines) {
        for (let i = 1; i < line.length; ++i) {
            // merge box in word if:
            // - same height
            // - less than 2 in space
            // if (line[i].h == line[i-1].h) {
            let currentLine = line[i];
            let previousLine = line[i - 1];
            if (canMergeMarkup(previousLine, currentLine)) {
                if (currentLine.x - previousLine.x - previousLine.w <= mergeThreshold) {
                    // if same word but less than .7 of prev line
                    if (currentLine.h != 0 && currentLine.h < previousLine.h * .7) {
                        // and not starting with space
                        if (currentLine.str[0] == ' ') {
                            break;
                        }
                        if (currentLine.y > previousLine.y + previousLine.h * .2) {
                            currentLine.str = strToSubscriptString(currentLine.str);
                        }
                        else {
                            currentLine.str = strToPostScript(currentLine.str);
                        }
                    }
                    previousLine.w = currentLine.x + currentLine.w - previousLine.x;
                    previousLine.str += currentLine.str;
                    previousLine.strLength += currentLine.strLength;
                    previousLine.pageBbox = mergePageBbox(previousLine, currentLine);
                    line.splice(i, 1);
                    i--;
                }
                else if (currentLine.x - previousLine.x - previousLine.w < previousLine.w / previousLine.strLength) {
                    // merge if space between this word and previous is less than average
                    // character width (using previous word font size)
                    previousLine.w = currentLine.x + currentLine.w - previousLine.x;
                    if (!previousLine.str.endsWith(" ")) {
                        previousLine.str += " ";
                        previousLine.strLength += 1;
                    }
                    previousLine.str += currentLine.str;
                    previousLine.strLength += currentLine.strLength;
                    previousLine.pageBbox = mergePageBbox(previousLine, currentLine);
                    line.splice(i, 1);
                    i--;

                }
            }
            // }
        }
    }


    // check if we can merge the lines together
    for (let i = 1; i < lines.length - 1; i++) {
        let currentLine = lines[i];
        let previousLine = lines[i - 1];

        let previousLineMinY = Math.min(...previousLine.map(v => v.y));
        let previousLineMaxY = Math.max(...previousLine.map(v => v.y + v.h));
        let currentLineMinY = Math.min(...currentLine.map(v => v.y));
        let currentLineMaxY = Math.max(...currentLine.map(v => v.y + v.h));

        // does the 2 line overlap?
        if (previousLineMaxY > currentLineMinY && previousLineMinY < currentLineMaxY) {
            // check the bboxes of current line and prevline do not overlap
            let bboxOverlap = false;
            for (let bbox of currentLine) {
                for (let prevBbox of previousLine) {
                    if (bbox.x >= prevBbox.x && bbox.x <= prevBbox.x + prevBbox.w) {
                        bboxOverlap = true;
                        break;
                    }
                    if (prevBbox.x >= bbox.x && prevBbox.x <= bbox.x + bbox.w) {
                        bboxOverlap = true;
                        break;
                    }
                }
            }

            // merge if no overlap
            if (!bboxOverlap) {
                previousLine.push(...currentLine);
                previousLine.sort((a, b) => a.x - b.x);
                lines.splice(i--, 1);
            }
        }
    }

    for (let i = 1; i < lines.length; i++) {
        const yDelta = lines[i][0].y - lines[i - 1][0].y - lines[i - 1][0].h;
        // Calculate the number of blank lines to insert based on vertical spacing
        // Use medianHeight as a reference for one line spacing
        if (yDelta > medianHeight) {
            // Calculate how many blank lines should be inserted
            // Round to nearest integer to get approximate number of lines
            const numBlankLines = Math.round(yDelta / medianHeight) - 1;
            // Cap at a reasonable maximum (e.g., 10 blank lines) to avoid extreme cases
            const linesToInsert = Math.min(Math.max(numBlankLines, 1), 10);
            
            // Insert the calculated number of blank lines
            const blankLines = Array(linesToInsert).fill([]);
            lines.splice(i, 0, ...blankLines);
            i += linesToInsert;
        }
    }
    return lines;
}

function canRenderBbox(line: ProjectionTextBox[], bbox: ProjectionTextBox): boolean {
    for (let item of line) {
        if (item == bbox) {
            return true;
        }
        if (!item.rendered) {
            return false;
        }
    }
    return false;
}

function updateForwardAnchorRightBound(snapMap: number[], forwardAnchor: ForwardAnchor, rightBound: number, anchorTarget: number): void {
    // Anything snapped to the right of rightBound should be aligned to anchorTarget line length at minimum
    for (let i = snapMap.length - 1; i >= 0; --i) {
        const anchor = snapMap[i];
        if (rightBound <= anchor) {
            if (!forwardAnchor[anchor] || anchorTarget > forwardAnchor[anchor]) {
                forwardAnchor[anchor] = anchorTarget;
            }
        }
        else {
            return;
        }
    }
}

function updateForwardAnchors(bbox: ProjectionTextBox, nextBbox: ProjectionTextBox | null, snapMaps: SnapMaps, forwardAnchors: PageForwardAnchors, lineLength: number): void {
    const rightBound = bbox.x + bbox.w;
    let targetLength = lineLength;
    if (nextBbox && (nextBbox.shouldSpace ?? 0) > 0) {
        targetLength += (nextBbox.shouldSpace ?? 0);
    }
    updateForwardAnchorRightBound(snapMaps.left, forwardAnchors.left, rightBound, targetLength);
    updateForwardAnchorRightBound(snapMaps.right, forwardAnchors.right, rightBound, targetLength);
    // we do not update center anchors since centered text may span between snapped columns
    updateForwardAnchorRightBound(snapMaps.floating, forwardAnchors.floating, rightBound, targetLength);
}

function resetBlockForwardAnchors(forwardAnchors: PageForwardAnchors): void {
    forwardAnchors.left = {};
    forwardAnchors.right = {};
    forwardAnchors.center = {};
    forwardAnchors.floating = {};
}

function getMedianTextBoxSize (lines: ProjectionTextBox[]): TextBoxSize {
    // calculate median textBox width
    let widthList = [];
    for (let bbox of lines) {
        if (bbox.w > 0) {
            widthList.push(bbox.w / bbox.strLength);
        }
    }
    const medianWidth = widthList.sort((a, b) => a - b)[Math.floor(widthList.length / 2)];

    // calculate median textBox height
    let heightList = [];
    for (let bbox of lines) {
        if (bbox.h > 0) {
            heightList.push(bbox.h);
        }
    }
    const medianHeight = heightList.sort((a, b) => a - b)[Math.floor(heightList.length / 2)];
    return {width: medianWidth, height: medianHeight};
}

export function projectToGrid(config: LiteParseConfig, page: PageData, projectionBoxes: ProjectionTextBox[], prevAnchors: PrevAnchors, totalPages: number): { text: string, prevAnchors: PrevAnchors } {

    // detect '.' garbage in the lines
    let dotCount = 0;
    for (let bbox of projectionBoxes) {
        // check if bbox.str contains only dots
        if (bbox.str.match(/^\.+$/)) {
            dotCount++;
        }
    }

    if (dotCount > 100 && dotCount > projectionBoxes.length * 0.05) {
        // remove all dots and splice them from lines
        let newLines = [];
        for (let bbox of projectionBoxes) {
            if (bbox.str.match(/^\.+$/)) {
                continue;
            }
            if (bbox.str.match(/^·+$/)) {
                continue;
            }
            if (bbox.str.match(/^"+$/)) {
                continue;
            }
            newLines.push(bbox);
        }
        projectionBoxes = newLines;
    }

    // calculate median textBox width/height
    const pageMedianSizes = getMedianTextBoxSize(projectionBoxes);
    let medianWidth = pageMedianSizes.width;
    let medianHeight = pageMedianSizes.height;

    let { verticalLines, horizontalLines } = getVerticalAndHorizontalLinesFromPagePaths(config, page.paths);
    // filter vertical and horizontal lines are less than 80% of page.width / height
    verticalLines = verticalLines.filter(line => line.height > page.height * 0.8);
    horizontalLines = horizontalLines.filter(line => line.width > page.width * 0.8);
    
    // Save original bboxes (including OCR) for text attribution
    const attributionBboxes: ProjectionTextBox[] = []
    for (const bbox of (projectionBoxes as ProjectionTextBox[])) {
        if (!bbox || !bbox.str || bbox.vgap || bbox.isPlaceholder) {
            continue;
        }
        attributionBboxes.push({
            str: bbox.str,
            x: bbox.x,
            y: bbox.y,
            w: bbox.w,
            h: bbox.h,
            r: bbox.r,
            strLength: bbox.str.length
        });
    }

    handleRotationReadingOrder(projectionBoxes, page.height);
    let lines = bboxToLine(projectionBoxes, medianWidth, medianHeight);

    // remove unprojectable text and apply markup to final lines
    for (let i = 0; i < lines.length; ++i) {
        const line = filterUnprojectableText(config, lines[i]);
        for (const bbox of line) {
            // With the way our grid projection currently works, we have to output
            // tags before raw line projection to avoid breaking the projection alignment.
            // The tags get replaced with MD as needed in output formatting, this does
            // result in output text containing the ~~ strikeout markup, but this is
            // mitigated since we skip markup entirely when we are not outputting markdown
            if (bbox.str.trim().length != 0 && bbox.markup) {
                bbox.str = applyMarkupTags(bbox.markup, bbox.str);
            }
        }
        lines[i] = line;
    }

    const forwardAnchors: PageForwardAnchors = {
        left: {},
        right: {},
        center: {},
        floating: {},
    };

    let rawLines: string[] = [];
    let rawLinesDelta = [];

    const blocks: LineRange[] = [];
    if (config.preserveLayoutAlignmentAcrossPages && totalPages > 1) {
        blocks.push({start: 0, end: lines.length});
    }
    else {
        let emptyCount = 0;
        let start = -1;
        for (const [lineIndex, line] of lines.entries()) {
            if (line.length === 0) {
                emptyCount++;
                if (emptyCount > 1) {
                    if (start >= 0) {
                        // ignore completely empty blocks, include the double blank
                        // line at the end of valid blocks
                        blocks.push({start: start, end: lineIndex + 1});
                    }
                    start = -1;
                }
            }
            else {
                emptyCount = 0;
                if (start < 0) {
                    start = lineIndex;
                }
            }
        }
        if (start > -1) {
            blocks.push({start: start, end: lines.length});
        }
    }

    for (const block of blocks) {
        let {
            anchorLeft,
            anchorRight,
            anchorCenter
        } = extractAnchorsPointsFromLines(lines.slice(block.start, block.end), page, verticalLines);

        const snapMaps: SnapMaps = {
            left: [],
            right: [],
            center: [],
            floating: [],
        }

        const uniqueSnaps = new Set<number>();
        for (let snap in anchorLeft) {
            uniqueSnaps.add(parseFloat(snap));
        }
        snapMaps.left.push(...uniqueSnaps);
        uniqueSnaps.clear()

        for (let snap in anchorRight) {
            uniqueSnaps.add(parseFloat(snap));
        }
        snapMaps.right.push(...uniqueSnaps);
        uniqueSnaps.clear()

        for (let snap in anchorCenter) {
            uniqueSnaps.add(parseFloat(snap));
        }
        snapMaps.center.push(...uniqueSnaps);
        uniqueSnaps.clear()

        let hasChanged = true;
        let leftSnap: Snap[] = [];
        let rightSnap: Snap[] = [];
        let centerSnap: Snap[] = [];

        if (!config.preserveLayoutAlignmentAcrossPages) {
            resetBlockForwardAnchors(forwardAnchors);

            const sizes = getMedianTextBoxSize(lines.slice(block.start, block.end).flat());
            medianWidth = sizes.width;
            medianHeight = sizes.height;
        }

        // compute snaps
        for (let lineIndex = block.start; lineIndex < block.end; ++lineIndex) {
            const line = lines[lineIndex];
            const forceUnsnapped = !canSnapLine(config, line);
            let prevBbox = null;

            for (let boxIndex = 0; boxIndex < line.length; ++boxIndex) {
                const bbox = line[boxIndex];
                bbox.forceUnsnapped = forceUnsnapped;

                const spaceThreshold = 2;
                // should we add a space between the two bbox?
                // TODO RTL
                if (prevBbox && bbox.x - (prevBbox.x + prevBbox.w) > spaceThreshold) {
                    // add a space
                    bbox.shouldSpace = 1;
                    const xDelta = bbox.x - (prevBbox.x + prevBbox.w);
                    const prevCharWidth = prevBbox.w / prevBbox.strLength;
                    if (xDelta > prevCharWidth * 2) {
                        // insert column spacing if any of:
                        // - gap is more than an approximate tab (8x average char width)
                        // - previous bbox is right snap
                        // - this bbox is left snap
                        // - both previous and this bbox are snaps
                        // otherwise insert floating spacing
                        if (!bbox.forceUnsnapped && (xDelta > prevCharWidth * 8) || (bbox.snap && bbox.snap === "left") || (prevBbox.snap && prevBbox.snap === "right") || (bbox.snap && prevBbox.snap)) {
                            bbox.shouldSpace = COLUMN_SPACES;
                        }
                        else {
                            bbox.shouldSpace = FLOATING_SPACES;
                        }
                    }
                }
                else {
                    bbox.shouldSpace = 0;
                }

                prevBbox = bbox;
                if (!bbox.snap) {
                    uniqueSnaps.add(Math.round(bbox.x));
                }
                else if (bbox.snap == 'left') {
                    leftSnap.push({ bbox, lineIndex, boxIndex });
                }
                else if (bbox.snap == 'right') {
                    rightSnap.push({ bbox, lineIndex, boxIndex });
                }
                else if (bbox.snap == 'center') {
                    centerSnap.push({ bbox, lineIndex, boxIndex });
                }
            }
        }

        snapMaps.floating.push(...uniqueSnaps);
        uniqueSnaps.clear()

        snapMaps.floating.sort((a, b) => a - b);
        snapMaps.center.sort((a, b) => a - b);
        snapMaps.right.sort((a, b) => a - b);
        snapMaps.left.sort((a, b) => a - b);

        while (hasChanged
            || snapMaps.right.length
            || snapMaps.left.length
            || snapMaps.center.length) {
            hasChanged = false;

            for (let lineIndex = block.start; lineIndex < block.end; ++lineIndex) {
                const line = lines[lineIndex];
                if (!rawLines[lineIndex]) {
                    rawLines[lineIndex] = "";
                    rawLinesDelta[lineIndex] = 0;
                }

                for (let boxIndex = 0; boxIndex < line.length; ++boxIndex) {
                    const bbox = line[boxIndex];
                    if (bbox.rendered) {
                        continue;
                    }

                    if (!bbox.forceUnsnapped) {
                        if (bbox.snap) {
                            continue;
                        }

                        if ((snapMaps.left.length && snapMaps.left[0] < bbox.x)
                            || (snapMaps.right.length && snapMaps.right[0] < bbox.x)
                            || (snapMaps.center.length && snapMaps.center[0] < Math.round(bbox.x + bbox.w / 2))) {
                            continue;
                        }
                    }

                    if (!canRenderBbox(line, bbox)) {
                        break;
                    }

                    let targetX = Math.min(Math.round(bbox.x / medianWidth), COLUMN_SPACES);

                    let lastSnapLeft = 0;
                    for (let key in forwardAnchors.left) {
                        if (parseInt(key) <= bbox.x) {
                            lastSnapLeft = Math.max(lastSnapLeft, forwardAnchors.left[key]);
                        }
                    }
                    const lineMax = Math.max(lastSnapLeft, rawLines[lineIndex].trimEnd().length + (bbox.shouldSpace ?? 0));
                    if (targetX < lineMax) {
                        targetX = lineMax;
                    }

                    if (!bbox.forceUnsnapped) {
                        const floatingAnchor = forwardAnchors.floating[Math.round(bbox.x)];
                        if (floatingAnchor && targetX < floatingAnchor) {
                            targetX = floatingAnchor;
                        }
                    }

                    rawLines[lineIndex] = rawLines[lineIndex].trimEnd();
                    if (targetX > rawLines[lineIndex].length) {
                        rawLines[lineIndex] += ' '.repeat(targetX - rawLines[lineIndex].length);
                    }

                    rawLines[lineIndex] += bbox.str;

                    bbox.rendered = true;
                    hasChanged = true;

                    let nextBbox: ProjectionTextBox | null = null;
                    if (line.length > (boxIndex + 1)) {
                        nextBbox = line[boxIndex + 1];
                    }
                    if (!bbox.forceUnsnapped) {
                        updateForwardAnchors(bbox, nextBbox, snapMaps, forwardAnchors, rawLines[lineIndex].length);
                    }
                }
            }

            if (snapMaps.left.length
                && (!snapMaps.right.length || snapMaps.left[0] <= snapMaps.right[0])
                && (!snapMaps.center.length || snapMaps.left[0] <= snapMaps.center[0])) {
                let thisTurnSnap: Snap[] = [];
                for (let item of leftSnap) {
                    if (item.bbox.leftAnchor && parseFloat(item.bbox.leftAnchor) == snapMaps.left[0]) {
                        thisTurnSnap.push(item);
                    }
                }
                hasChanged = true;
                if (!thisTurnSnap.length) {
                    snapMaps.left.shift();
                    continue;
                }

                let targetX = Math.min(Math.round(snapMaps.left[0] / medianWidth), COLUMN_SPACES);
                let lineMax = Math.max(...thisTurnSnap.map((v) => {
                    let spaceEnd = 0;
                    if (!rawLines[v.lineIndex].endsWith(' ')) {
                        spaceEnd = v.bbox.shouldSpace ?? 0;
                    }
                    if ((v.bbox.shouldSpace ?? 0) > 1) {
                        const trailingSpaces = rawLines[v.lineIndex].length - rawLines[v.lineIndex].trimEnd().length;
                        if (trailingSpaces < (v.bbox.shouldSpace ?? 0)) {
                            spaceEnd = (v.bbox.shouldSpace ?? 0) - trailingSpaces;
                        }
                    }

                    return rawLines[v.lineIndex].length + spaceEnd + 1;
                }));

                if (targetX < lineMax) {
                    targetX = lineMax;
                }

                if (forwardAnchors.left[snapMaps.left[0]] && targetX < forwardAnchors.left[snapMaps.left[0]]) {
                    targetX = forwardAnchors.left[snapMaps.left[0]];
                }
                if (prevAnchors.forwardAnchorLeft[snapMaps.left[0]] && targetX < prevAnchors.forwardAnchorLeft[snapMaps.left[0]]) {
                    targetX = prevAnchors.forwardAnchorLeft[snapMaps.left[0]];
                }

                forwardAnchors.left[snapMaps.left[0]] = targetX;

                for (let currentLeftSnapBox of thisTurnSnap) {
                    const lineIndex = currentLeftSnapBox.lineIndex;
                    if (targetX > rawLines[lineIndex].length) {
                        rawLines[lineIndex] += ' '.repeat(targetX - rawLines[lineIndex].length);
                    }
                    rawLines[lineIndex] += currentLeftSnapBox.bbox.str;
                    currentLeftSnapBox.bbox.rendered = true;

                    let nextBbox: ProjectionTextBox | null = null;
                    if (lines[lineIndex].length > (currentLeftSnapBox.boxIndex + 1)) {
                        nextBbox = lines[lineIndex][currentLeftSnapBox.boxIndex + 1];
                    }
                    updateForwardAnchors(currentLeftSnapBox.bbox, nextBbox, snapMaps, forwardAnchors, rawLines[lineIndex].length);
                }

                for (let index = block.start; index < block.end; ++index) {
                    const line = rawLines[index];
                    if (line.length < targetX) {
                        rawLines[index] += ' '.repeat(targetX - line.length);
                    }
                }
                snapMaps.left.shift();
            }

            else if (snapMaps.right.length
                && (!snapMaps.left.length || snapMaps.right[0] <= snapMaps.left[0])
                && (!snapMaps.center.length || snapMaps.right[0] <= snapMaps.center[0])) {
                let thisTurnSnap: Snap[] = [];
                hasChanged = true;
                for (let item of rightSnap) {
                    if (item.bbox.rightAnchor && parseFloat(item.bbox.rightAnchor) == snapMaps.right[0]) {
                        thisTurnSnap.push(item);
                    }
                }

                if (!thisTurnSnap.length) {
                    snapMaps.right.shift();
                    continue;
                }

                let targetX = Math.min(Math.round(snapMaps.right[0] / medianWidth), COLUMN_SPACES);
                let lineMax = Math.max(...thisTurnSnap.map((v) => {
                    let lastSnapLeft = 0;
                    for (let key in forwardAnchors.left) {
                        if (parseInt(key) <= v.bbox.x) {
                            lastSnapLeft = Math.max(lastSnapLeft, forwardAnchors.left[key]);
                        }
                    }
                    return Math.max(lastSnapLeft, rawLines[v.lineIndex].trimEnd().length + (v.bbox.shouldSpace ?? 0)) + v.bbox.strLength
                }));

                if (targetX < lineMax) {
                    targetX = lineMax;
                }
                if (forwardAnchors.right[snapMaps.right[0]] && targetX < forwardAnchors.right[snapMaps.right[0]]) {
                    targetX = forwardAnchors.right[snapMaps.right[0]];
                }
                if (prevAnchors.forwardAnchorRight[snapMaps.right[0]] && targetX < prevAnchors.forwardAnchorRight[snapMaps.right[0]]) {
                    targetX = prevAnchors.forwardAnchorRight[snapMaps.right[0]];
                }
                forwardAnchors.right[snapMaps.right[0]] = targetX;

                for (let currentRightSnapBox of thisTurnSnap) {
                    const lineIndex = currentRightSnapBox.lineIndex;
                    rawLines[lineIndex] = rawLines[lineIndex].trimEnd();
                    if (targetX > rawLines[lineIndex].trimEnd().length + currentRightSnapBox.bbox.strLength) {
                        rawLines[lineIndex] += ' '.repeat(targetX - rawLines[lineIndex].length - currentRightSnapBox.bbox.strLength);
                    }
                    rawLines[lineIndex] += currentRightSnapBox.bbox.str;
                    currentRightSnapBox.bbox.rendered = true;

                    let nextBbox: ProjectionTextBox | null = null;
                    if (lines[lineIndex].length > (currentRightSnapBox.boxIndex + 1)) {
                        nextBbox = lines[lineIndex][currentRightSnapBox.boxIndex + 1];
                    }
                    updateForwardAnchors(currentRightSnapBox.bbox, nextBbox, snapMaps, forwardAnchors, rawLines[lineIndex].length);
                }
                for (let index = block.start; index < block.end; ++index) {
                    const line = rawLines[index];
                    if (line.length < targetX) {
                        rawLines[index] += ' '.repeat(targetX - line.length);
                    }
                }
                snapMaps.right.shift();
            }

            else if (snapMaps.center.length
                && (!snapMaps.left.length || snapMaps.center[0] <= snapMaps.left[0])
                && (!snapMaps.right.length || snapMaps.center[0] <= snapMaps.right[0])) {
                let thisTurnSnap: Snap[] = [];
                hasChanged = true;
                for (let item of centerSnap) {
                    if (item.bbox.centerAnchor && parseFloat(item.bbox.centerAnchor) == snapMaps.center[0]) {
                        thisTurnSnap.push(item);
                    }
                }
                if (!thisTurnSnap.length) {
                    snapMaps.center.shift();
                    continue;
                }
                let targetX = Math.min(Math.round(snapMaps.center[0] / medianWidth), COLUMN_SPACES);
                let lineMax = Math.max(
                    ...(thisTurnSnap.map((v) => {
                        let spaceEnd = 0;
                        if (!rawLines[v.lineIndex].endsWith(' ')) {
                            spaceEnd = v.bbox.shouldSpace ?? 0;
                        }
                        if ((v.bbox.shouldSpace ?? 0) > 1) {
                            const trailingSpaces = rawLines[v.lineIndex].length - rawLines[v.lineIndex].trimEnd().length;
                            if (trailingSpaces < (v.bbox.shouldSpace ?? 0)) {
                                spaceEnd = (v.bbox.shouldSpace ?? 0) - trailingSpaces;
                            }
                        }
                        return rawLines[v.lineIndex].length
                            + Math.round(v.bbox.strLength / 2)
                            + spaceEnd
                    }
                    )));

                if (targetX < lineMax) {
                    targetX = lineMax;
                }
                if (forwardAnchors.center[snapMaps.center[0]] && targetX < forwardAnchors.center[snapMaps.center[0]]) {
                    targetX = forwardAnchors.center[snapMaps.center[0]];
                }
                if (prevAnchors.forwardAnchorCenter[snapMaps.center[0]] && targetX < prevAnchors.forwardAnchorCenter[snapMaps.center[0]]) {
                    targetX = prevAnchors.forwardAnchorCenter[snapMaps.center[0]];
                }
                forwardAnchors.center[snapMaps.center[0]] = targetX;
                for (let currentCenterSnapBox of thisTurnSnap) {
                    if (targetX > rawLines[currentCenterSnapBox.lineIndex].length + Math.round(currentCenterSnapBox.bbox.strLength / 2)) {
                        rawLines[currentCenterSnapBox.lineIndex] += ' '.repeat(targetX - rawLines[currentCenterSnapBox.lineIndex].length - Math.round(currentCenterSnapBox.bbox.strLength / 2));
                    }
                    rawLines[currentCenterSnapBox.lineIndex] += currentCenterSnapBox.bbox.str;
                    currentCenterSnapBox.bbox.rendered = true;
                }
                snapMaps.center.shift();
            }
        }
    }

    fixSparseBlocks(blocks, rawLines);

    let text = rawLines.join("\n");
    // OSS: Return text instead of mutating page object
    return {
        text,
        prevAnchors: {
            forwardAnchorLeft: forwardAnchors.left,
            forwardAnchorRight: forwardAnchors.right,
            forwardAnchorCenter: forwardAnchors.center,
        }
    };
}

export function projectPagesToGrid(pages: PageData[], config: LiteParseConfig): ParsedPage[] {
    let prevAnchors: PrevAnchors = {
        forwardAnchorLeft: {},
        forwardAnchorRight: {},
        forwardAnchorCenter: {}
    };

    const results: ParsedPage[] = [];

    for (const page of pages) {
        // Build projection boxes from text items
        const projectionBoxes = buildBbox(page, config);

        // Project to grid
        const { text, prevAnchors: newAnchors } = projectToGrid(config, page, projectionBoxes, prevAnchors, pages.length);

        // Update forward anchors if preserving across pages
        if (config.preserveLayoutAlignmentAcrossPages) {
            for (let anchor in newAnchors.forwardAnchorLeft) {
                prevAnchors.forwardAnchorLeft[anchor] = newAnchors.forwardAnchorLeft[anchor];
            }
            for (let anchor in newAnchors.forwardAnchorRight) {
                prevAnchors.forwardAnchorRight[anchor] = newAnchors.forwardAnchorRight[anchor];
            }
            for (let anchor in newAnchors.forwardAnchorCenter) {
                prevAnchors.forwardAnchorCenter[anchor] = newAnchors.forwardAnchorCenter[anchor];
            }
        }

        // Build result page
        results.push({
            pageNum: page.pageNum,
            width: page.width,
            height: page.height,
            text,
            textItems: page.textItems,
            boundingBoxes: [],
            tables: [],
            images: []
        });
    }

    // Clean raw text (margin detection, etc)
    cleanRawText(results, config);

    return results;
}
