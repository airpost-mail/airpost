// SPDX-License-Identifier: GPL-3.0-only
import React from 'react';
import * as TabsPrimitive from '@radix-ui/react-tabs';

/* Public: Tabs on Radix Tabs with shadcn's parts (docs/desktop/ui-kit.md 6.13). Values are
   strings; arrow keys follow the UI direction.

   ```
   <Tabs defaultValue="people">
     <TabsList aria-label="Emoji">
       <TabsTrigger value="people">People</TabsTrigger>
       <TabsTrigger value="nature">Nature</TabsTrigger>
     </TabsList>
     <TabsContent value="people">…</TabsContent>
     <TabsContent value="nature">…</TabsContent>
   </Tabs>
   ```
*/

export const Tabs = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Root>
>(function Tabs(props, ref) {
  return <TabsPrimitive.Root ref={ref} data-slot="tabs" {...props} />;
});

export const TabsList = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.List>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.List>
>(function TabsList(props, ref) {
  return <TabsPrimitive.List ref={ref} data-slot="tabs-list" {...props} />;
});

export const TabsTrigger = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Trigger>
>(function TabsTrigger(props, ref) {
  return <TabsPrimitive.Trigger ref={ref} data-slot="tabs-trigger" {...props} />;
});

export const TabsContent = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Content>
>(function TabsContent(props, ref) {
  return <TabsPrimitive.Content ref={ref} data-slot="tabs-content" {...props} />;
});

Tabs.displayName = 'Tabs';
TabsList.displayName = 'TabsList';
TabsTrigger.displayName = 'TabsTrigger';
TabsContent.displayName = 'TabsContent';
