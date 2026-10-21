/*
 * decaffeinate suggestions:
 * DS205: Consider reworking code to avoid use of IIFEs
 * DS207: Consider shorter variations of null checks
 * Full docs: https://github.com/decaffeinate/decaffeinate/blob/master/docs/suggestions.md
 */
const ItemSpecificities = new WeakMap<IMenuItem, number>();

export type IMenuItem = {
  label?: string;
  submenu?: IMenuItem[];
  type?: 'separator' | 'checkbox';

  id?: string; //unlocalized label
  command?: string;
  enabled?: boolean;
  hideWhenDisabled?: boolean;
  visible?: boolean;
  isOptional?: boolean;
};

type MergeResult = IMenuItem | number | MergeResult[] | undefined;

// findMatchingItemIndex only matches an item with a submenu to one with a submenu.
export function merge(menu: IMenuItem[], item: IMenuItem, itemSpecificity?: number): MergeResult {
  let matchingItem: IMenuItem | undefined;
  if (itemSpecificity == null) {
    itemSpecificity = Infinity;
  }
  item = cloneMenuItem(item);
  if (itemSpecificity) {
    ItemSpecificities.set(item, itemSpecificity);
  }
  const matchingItemIndex = findMatchingItemIndex(menu, item);
  if (matchingItemIndex !== -1) {
    matchingItem = menu[matchingItemIndex];
  }

  if (matchingItem != null) {
    if (item.submenu != null) {
      const matchingSubmenu = matchingItem.submenu!;
      return item.submenu.map((submenuItem) =>
        merge(matchingSubmenu, submenuItem, itemSpecificity)
      );
    } else if (itemSpecificity) {
      const matchingSpecificity = ItemSpecificities.get(matchingItem);
      if (matchingSpecificity === undefined || !(itemSpecificity < matchingSpecificity)) {
        return (menu[matchingItemIndex] = item);
      }
    }
  } else if (item.type !== 'separator' || (menu.at(-1) || ({} as any)).type !== 'separator') {
    return menu.push(item);
  }
}

export function unmerge(menu: IMenuItem[], item: IMenuItem) {
  let matchingItem: IMenuItem | undefined;
  const matchingItemIndex = findMatchingItemIndex(menu, item);
  if (matchingItemIndex !== -1) {
    matchingItem = menu[matchingItemIndex];
  }

  if (matchingItem != null) {
    if (item.submenu != null) {
      const matchingSubmenu = matchingItem.submenu!;
      for (const submenuItem of Array.from(item.submenu)) {
        unmerge(matchingSubmenu, submenuItem);
      }
    }

    if (
      (matchingItem.submenu != null ? matchingItem.submenu : []).length === 0 ||
      matchingItem.isOptional
    ) {
      return menu.splice(matchingItemIndex, 1);
    }
  }
}

export function findMatchingItemIndex(menu: IMenuItem[], { type, label, submenu }: IMenuItem) {
  if (type === 'separator') {
    return -1;
  }
  for (let index = 0; index < menu.length; index++) {
    const item = menu[index];
    if (
      normalizeLabel(item.label) === normalizeLabel(label) &&
      (item.submenu != null) === (submenu != null)
    ) {
      return index;
    }
  }
  return -1;
}

export function normalizeLabel(label: string | null | undefined) {
  if (label == null) {
    return undefined;
  }

  if (process.platform === 'darwin') {
    return label;
  } else {
    return label.replace(/&/g, '');
  }
}

export function cloneMenuItem(item: IMenuItem) {
  item = { ...item };
  if (item.submenu != null) {
    item.submenu = item.submenu.map((submenuItem) => cloneMenuItem(submenuItem));
  }
  return item;
}

export function forEachMenuItem(menu: IMenuItem[], callback: (item: IMenuItem) => void) {
  const result = [];
  for (const item of Array.from(menu)) {
    if (item.submenu != null) {
      forEachMenuItem(item.submenu, callback);
    }
    result.push(callback(item));
  }
  return result;
}
