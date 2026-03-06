import { UserService } from './utils';
import { User, ApiResponse } from './types';

const API_VERSION = "1.0.0";
const MAX_PAGE_SIZE = 100;

export class App {
  private userService: UserService;
  private running: boolean;

  constructor() {
    this.userService = new UserService();
    this.running = false;
  }

  async start(): Promise<void> {
    this.running = true;
    console.log(`App v${API_VERSION} started`);
    await this.userService.initialize();
  }

  async stop(): Promise<void> {
    this.running = false;
    console.log("App stopped");
  }

  async getUsers(page: number = 1): Promise<ApiResponse<User[]>> {
    const users = await this.userService.list(page, MAX_PAGE_SIZE);
    return {
      data: users,
      status: 200,
      message: "OK",
    };
  }
}

export function createApp(): App {
  return new App();
}
