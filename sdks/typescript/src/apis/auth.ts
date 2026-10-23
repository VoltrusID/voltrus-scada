import type { HttpClient } from '../http';
import type {
  AuthMe,
  AuthProvider,
  ChangePasswordRequest,
  LoginRequest,
  LoginResponse,
} from '../types';

export class AuthApi {
  constructor(private readonly http: HttpClient) {}

  /**
   * Local username/password login. The server delivers the session as
   * HttpOnly Set-Cookie headers (voltrus_token / voltrus_refresh) and does
   * NOT return a token in the body — this client captures and replays the
   * cookies for you, so call this before any protected request.
   */
  async login(username: string, password: string): Promise<LoginResponse> {
    const body: LoginRequest = { username, password };
    const res = await this.http.postRaw('/api/v1/auth/login', body);
    this.http.captureSessionCookies(res);
    return res.data as LoginResponse;
  }

  /** Rotate the session using the stored voltrus_refresh cookie. */
  async refreshToken(): Promise<{ ok: boolean }> {
    const res = await this.http.postRaw('/api/v1/auth/refresh');
    this.http.captureSessionCookies(res);
    return res.data as { ok: boolean };
  }

  async logout(): Promise<void> {
    await this.http.post('/api/v1/auth/logout');
    this.http.clearSessionCookies();
  }

  /** Identify the current session (cookie or API key). */
  async me(): Promise<AuthMe> {
    return this.http.get<AuthMe>('/api/v1/auth/me');
  }

  async listProviders(): Promise<{ providers: AuthProvider[] }> {
    return this.http.get('/api/v1/auth/providers');
  }

  async getCsrfToken(): Promise<{ csrf_token: string }> {
    return this.http.get('/api/v1/auth/csrf-token');
  }

  async changePassword(oldPassword: string, newPassword: string): Promise<{ ok: boolean }> {
    const body: ChangePasswordRequest = { old_password: oldPassword, new_password: newPassword };
    return this.http.post('/api/v1/auth/change-password', body);
  }
}
