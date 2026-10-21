import React from 'react';
import { MailboxPerspective } from '../../data/mailbox-perspective';

export interface ISidebarItem {
  id: string;
  name?: string;
  contextMenuLabel?: string;
  count: number;
  iconName?: string;
  iconColor?: string;
  children: ISidebarItem[];
  perspective: MailboxPerspective;
  selected: boolean;
  collapsed: boolean;
  counterStyle?: string;
  onDelete?: () => void;
  onEdited?: (item: ISidebarItem, name: string) => void;
  onExport?: () => void;
  onExportMbox?: () => void;
  onMarkAllAsRead?: (item: ISidebarItem) => void;
  onCreateChild?: (item: ISidebarItem, childName: string) => void;
  onEditColor?: (item: ISidebarItem, originRect: DOMRect) => void;
  onCollapseToggled: () => void;
  onDrop: (item: ISidebarItem, event: React.DragEvent) => void;
  shouldAcceptDrop: (item: ISidebarItem, event: React.DragEvent) => boolean;
  onSelect: (item: ISidebarItem) => void;

  deletable?: boolean;
  editable?: boolean;
  exportable?: boolean;
  deleted?: boolean;
}

export interface ISidebarSection {
  title: string;
  items: ISidebarItem[];
  iconName?: string;
  collapsed?: boolean;
  titleColor?: string;
  onCollapseToggled?: (section: { title: string }) => void;
  onItemCreated?: (displayName: string) => void;
}
