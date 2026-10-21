import { Category } from '../../data/models/category';

export type CategoryData = {
  id?: string;
  divider?: boolean;
  newCategoryItem?: boolean;
  searchValue?: string;
  category?: Category;
  displayName?: string;
  backgroundColor?: string;
  usage?: number;
  numThreads?: number;
  name?: string;
};
