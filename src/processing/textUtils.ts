/**
 * Convert string to subscript unicode characters
 */
export function strToSubscriptString(str: string): string {
  const sub: { [key: string]: string } = {
    '0': '₀',
    '1': '₁',
    '2': '₂',
    '3': '₃',
    '4': '₄',
    '5': '₅',
    '6': '₆',
    '7': '₇',
    '8': '₈',
    '9': '₉',
    '+': '₊',
    '-': '₋',
    'a': 'ₐ',
    'e': 'ₑ',
    'o': 'ₒ',
    'x': 'ₓ',
    'ə': 'ₔ',
    'h': 'ₕ',
    'k': 'ₖ',
    'l': 'ₗ',
    'm': 'ₘ',
    'n': 'ₙ',
    'p': 'ₚ',
    'r': 'ᵣ',
    's': 'ₛ',
    't': 'ₜ',
  };

  let subStr = '';
  for (let i = 0; i < str.length; i++) {
    if (!sub[str[i]]) {
      return str;
    }
    subStr += sub[str[i]];
  }

  return subStr;
}

/**
 * Convert string to superscript unicode characters
 */
export function strToPostScript(str: string): string {
  const post: { [key: string]: string } = {
    '0': '⁰',
    '1': '¹',
    '2': '²',
    '3': '³',
    '4': '⁴',
    '5': '⁵',
    '6': '⁶',
    '7': '⁷',
    '8': '⁸',
    '9': '⁹',
    '+': '⁺',
    '-': '⁻',
    'a': 'ᵃ',
    'b': 'ᵇ',
    'c': 'ᶜ',
    'd': 'ᵈ',
    'e': 'ᵉ',
    'f': 'ᶠ',
    'g': 'ᵍ',
    'h': 'ʰ',
    'i': 'ⁱ',
    'j': 'ʲ',
    'k': 'ᵏ',
    'l': 'ˡ',
    'm': 'ᵐ',
    'n': 'ⁿ',
    'o': 'ᵒ',
    'p': 'ᵖ',
    'r': 'ʳ',
    's': 'ˢ',
    't': 'ᵗ',
    'u': 'ᵘ',
    'v': 'ᵛ',
    'w': 'ʷ',
    'x': 'ˣ',
    'y': 'ʸ',
    'z': 'ᶻ',
    'A': 'ᴬ',
    'B': 'ᴮ',
    'D': 'ᴰ',
    'E': 'ᴱ',
    'G': 'ᴳ',
    'H': 'ᴴ',
    'I': 'ᴵ',
    'J': 'ᴶ',
    'K': 'ᴷ',
    'L': 'ᴸ',
    'M': 'ᴹ',
    'N': 'ᴺ',
    'O': 'ᴼ',
    'P': 'ᴾ',
    'R': 'ᴿ',
    'T': 'ᵀ',
    'U': 'ᵁ',
    'V': 'ⱽ',
    'W': 'ᵂ',
  };

  let postStr = '';
  for (let i = 0; i < str.length; i++) {
    if (!post[str[i]]) {
      return str;
    }
    postStr += post[str[i]];
  }

  return postStr;
}
