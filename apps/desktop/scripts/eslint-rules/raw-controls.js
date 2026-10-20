// SPDX-License-Identifier: GPL-3.0-only
// Warns on controls drawn without the UI kit (src/components/kit, docs/desktop/ui-kit.md): raw
// <button> and <select>, <input type="checkbox|radio">, and the button classes of
// static/styles/buttons.less (`.btn`, `.btn-toolbar`, …) on DOM elements. On kit components such
// a class is a hook for themes and plugins and isn't reported. Loaded through `--rulesdir scripts/eslint-rules` (package.json lint scripts).

const KIT = {
  button: 'Button or IconButton (components/kit/button, icon-button)',
  select: 'Select (components/kit/select) or SettingsSelect (components/settings-select)',
  checkbox: 'Checkbox (components/kit/checkbox)',
  radio: 'RadioGroup (components/kit/radio-group)',
  btn: 'Button or IconButton (components/kit/button, icon-button)',
};

// The button looks static/styles/buttons.less defines.
const BTN_CLASS =
  /^btn(-(action|danger|destructive|disabled|emphasis|gradient|icon|large|larger|small|toolbar))?$/;

function elementName(node) {
  return node.name && node.name.type === 'JSXIdentifier' ? node.name.name : null;
}

function attribute(node, name) {
  return node.attributes.find((a) => a.type === 'JSXAttribute' && a.name && a.name.name === name);
}

function literalValue(attr) {
  if (!attr || !attr.value) return null;
  if (attr.value.type === 'Literal') return attr.value.value;
  const expr = attr.value.type === 'JSXExpressionContainer' ? attr.value.expression : null;
  if (expr && expr.type === 'Literal') return expr.value;
  if (expr && expr.type === 'TemplateLiteral' && expr.expressions.length === 0) {
    return expr.quasis[0].value.cooked;
  }
  return null;
}

// Every class-name string an expression can produce: string literals, template literal parts,
// and the keys of classnames() objects.
function classStrings(node, out = []) {
  if (!node) return out;
  switch (node.type) {
    case 'Literal':
      if (typeof node.value === 'string') out.push(node.value);
      break;
    case 'TemplateLiteral':
      node.quasis.forEach((q) => out.push(q.value.cooked || ''));
      node.expressions.forEach((e) => classStrings(e, out));
      break;
    case 'JSXExpressionContainer':
      classStrings(node.expression, out);
      break;
    case 'ConditionalExpression':
      classStrings(node.consequent, out);
      classStrings(node.alternate, out);
      break;
    case 'LogicalExpression':
    case 'BinaryExpression':
      classStrings(node.left, out);
      classStrings(node.right, out);
      break;
    case 'CallExpression':
      node.arguments.forEach((a) => classStrings(a, out));
      break;
    case 'ArrayExpression':
      node.elements.forEach((e) => classStrings(e, out));
      break;
    case 'ObjectExpression':
      node.properties.forEach((p) => {
        if (p.type !== 'Property') return;
        if (p.key.type === 'Identifier' && !p.computed) out.push(p.key.name);
        else classStrings(p.key, out);
      });
      break;
    default:
      break;
  }
  return out;
}

module.exports = {
  meta: {
    type: 'suggestion',
    docs: { description: 'Use the UI kit instead of raw buttons, selects and toggles.' },
    schema: [],
    messages: {
      raw: 'Raw {{what}} outside the UI kit; use the kit {{kit}}.',
    },
  },
  create(context) {
    return {
      JSXOpeningElement(node) {
        const name = elementName(node);
        if (!name || name[0] !== name[0].toLowerCase()) return;

        if (name === 'button' || name === 'select') {
          context.report({ node, messageId: 'raw', data: { what: `<${name}>`, kit: KIT[name] } });
        } else if (name === 'input') {
          const type = literalValue(attribute(node, 'type'));
          if (type === 'checkbox' || type === 'radio') {
            context.report({
              node,
              messageId: 'raw',
              data: { what: `<input type="${type}">`, kit: KIT[type] },
            });
          }
        }

        const className = attribute(node, 'className');
        if (className) {
          const btn = classStrings(className.value)
            .flatMap((s) => s.split(/\s+/))
            .find((c) => BTN_CLASS.test(c));
          if (btn) {
            context.report({
              node: className,
              messageId: 'raw',
              data: { what: `.${btn} class`, kit: KIT.btn },
            });
          }
        }
      },
    };
  },
};
