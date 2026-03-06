import { User } from './types';

export class UserService {
  private users: User[];

  constructor() {
    this.users = [];
  }

  async initialize(): Promise<void> {
    this.users = [
      { id: 1, name: "Alice", email: "alice@example.com", role: "admin" },
      { id: 2, name: "Bob", email: "bob@example.com", role: "user" },
    ];
  }

  async list(page: number, pageSize: number): Promise<User[]> {
    const start = (page - 1) * pageSize;
    return this.users.slice(start, start + pageSize);
  }

  async findById(id: number): Promise<User | undefined> {
    return this.users.find(u => u.id === id);
  }

  async create(user: Omit<User, 'id'>): Promise<User> {
    const newUser: User = {
      ...user,
      id: this.users.length + 1,
    };
    this.users.push(newUser);
    return newUser;
  }
}

export function formatUser(user: User): string {
  return `${user.name} <${user.email}> [${user.role}]`;
}

export function validateEmail(email: string): boolean {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email);
}
