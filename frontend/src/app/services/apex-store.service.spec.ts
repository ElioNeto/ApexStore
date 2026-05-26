import { TestBed } from '@angular/core/testing';
import { HttpClientTestingModule, HttpTestingController } from '@angular/common/http/testing';
import { ApexStoreService } from './apex-store.service';
import { environment } from '../../environments/environment';

describe('ApexStoreService', () => {
  let service: ApexStoreService;
  let httpMock: HttpTestingController;

  beforeEach(() => {
    TestBed.configureTestingModule({
      imports: [HttpClientTestingModule],
      providers: [ApexStoreService],
    });
    service = TestBed.inject(ApexStoreService);
    httpMock = TestBed.inject(HttpTestingController);
  });

  afterEach(() => {
    httpMock.verify();
  });

  it('should be created', () => {
    expect(service).toBeTruthy();
  });

  describe('put()', () => {
    it('should POST to /keys with key and value', () => {
      service.put('test-key', 'test-value').subscribe(res => {
        expect(res.success).toBeTrue();
        expect(res.data?.key).toBe('test-key');
      });

      const req = httpMock.expectOne(`${environment.apiUrl}/keys`);
      expect(req.request.method).toBe('POST');
      expect(req.request.body).toEqual({ key: 'test-key', value: 'test-value' });
      req.flush({ success: true, data: { key: 'test-key' } });
    });
  });

  describe('get()', () => {
    it('should GET from /keys/{key}', () => {
      service.get('my-key').subscribe(res => {
        expect(res.key).toBe('my-key');
        expect(res.value).toBe('my-value');
      });

      const req = httpMock.expectOne(`${environment.apiUrl}/keys/${encodeURIComponent('my-key')}`);
      expect(req.request.method).toBe('GET');
      req.flush({ success: true, data: { key: 'my-key', value: 'my-value' } });
    });
  });

  describe('delete()', () => {
    it('should DELETE to /keys/{key}', () => {
      service.delete('key-to-delete').subscribe(res => {
        expect(res.success).toBeTrue();
      });

      const req = httpMock.expectOne(`${environment.apiUrl}/keys/${encodeURIComponent('key-to-delete')}`);
      expect(req.request.method).toBe('DELETE');
      req.flush({ success: true, data: null });
    });
  });

  describe('listKeys()', () => {
    it('should GET /keys and return key list', () => {
      service.listKeys().subscribe(keys => {
        expect(keys).toEqual(['a', 'b']);
      });

      const req = httpMock.expectOne(`${environment.apiUrl}/keys`);
      expect(req.request.method).toBe('GET');
      req.flush({ success: true, data: { keys: ['a', 'b'] } });
    });
  });

  describe('getStats()', () => {
    it('should GET /stats/all and return stats object', () => {
      service.getStats().subscribe(stats => {
        expect(stats['mem_records']).toBe(100);
      });

      const req = httpMock.expectOne(`${environment.apiUrl}/stats/all`);
      expect(req.request.method).toBe('GET');
      req.flush({ success: true, data: { mem_records: 100 } });
    });
  });
});
