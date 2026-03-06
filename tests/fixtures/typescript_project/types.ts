export interface User {
  id: number;
  name: string;
  email: string;
  role: "admin" | "user" | "moderator";
}

export interface ApiResponse<T> {
  data: T;
  status: number;
  message: string;
}

export type UserRole = "admin" | "user" | "moderator";

export interface PaginationOptions {
  page: number;
  pageSize: number;
  sortBy?: string;
  sortOrder?: "asc" | "desc";
}

export const DEFAULT_PAGE_SIZE = 20;
