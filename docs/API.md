# Android Device Desk 로컬 API / Local API

기기 데스크의 오른쪽 **로컬 API → 서버 켜기 → API 토큰 복사**를 누릅니다. 같은 PC에서 `http://127.0.0.1:17653`로 접속하고 `Authorization: Bearer <token>` 헤더를 붙입니다. 서버를 다시 켜면 새 토큰이 발급됩니다.

In Device Desk, choose **Local API → Start server → Copy API token**. Send an `Authorization: Bearer <token>` header to `http://127.0.0.1:17653` from the same PC. Restarting the server generates a fresh token.

## 엔드포인트 / Endpoints

| Method | Path | 설명 / Description |
| --- | --- | --- |
| GET | `/v1/devices` | 기기 번호·이름·상태·제어 가능 여부 / Device number, name, connection state, and control availability |
| GET | `/v1/devices/{number}/apps` | 실행 가능한 앱의 패키지와 컴포넌트 / Launchable Android packages and activity components |
| POST | `/v1/actions` | 동작 목록 제출, HTTP 202와 `job_id` 반환 / Submit a plan and receive HTTP 202 with a `job_id` |
| GET | `/v1/presets` | 저장한 프리셋 ID·이름·단계 수 / Saved preset IDs, names, and step counts |
| POST | `/v1/presets/{id}/run` | 대상 기기에서 저장한 작업 실행 / Run a saved preset on the requested devices |
| POST | `/v1/jobs/{job_id}/retry` | 실패한 기기에서만 첫 단계부터 재실행 / Replay from step one on failed devices only |
| GET | `/v1/jobs/{job_id}` | 작업 상태와 기기별 완료 단계 조회 / Job status and per-device step results |

## 요청 형식 / Request format

`Content-Type: application/json`을 사용합니다. `targets`는 앱에 표시된 번호 하나(`2`), 번호 목록(`[2, 5]`), 연결된 제어 가능 기기 전체(`"all"`), 또는 기기 태그(`{"tag":"QA"}`)입니다.

Set `Content-Type: application/json`. Use a single device number such as `2`, an array such as `[2, 5]`, `"all"` for all currently connected and controllable devices, or a tag object such as `{"tag":"QA"}`. Tags match exactly.

```json
{
  "targets": [2, 5],
  "steps": [
    {"action": "open_url", "url": "https://example.com/"},
    {"action": "wait", "ms": 1000},
    {"action": "swipe", "x1": 0.5, "y1": 0.8, "x2": 0.5, "y2": 0.3, "duration_ms": 450}
  ]
}
```

| Action | Fields | 설명 / Description |
| --- | --- | --- |
| `open_url` | `url` | Android에 HTTP(S) 링크 열기 요청 / Ask Android to open an HTTP(S) link |
| `launch_app` | `package`, optional `component` | 앱 실행. 컴포넌트 예는 `com.example.app/.MainActivity` / Start an app, optionally by activity component |
| `tap` | `x`, `y` | 화면 탭 / Tap |
| `swipe` | `x1`, `y1`, `x2`, `y2`, `duration_ms` | 화면 스와이프 / Swipe. 50–3,000 ms |
| `key` | `key` | `home`, `back`, `recents`, `enter` |
| `wait` | `ms` | 1–15,000 ms 대기 / Pause for 1–15,000 ms |

좌표는 각 기기의 현재 화면에 대한 상대값입니다. `(0, 0)`은 왼쪽 위, `(1, 1)`은 오른쪽 아래입니다. 한 작업에 최대 32개 단계, 합계 60초의 명시적 대기를 넣을 수 있습니다. 웹사이트나 앱 내부 데이터가 준비되는 데 더 시간이 걸리면 대기 단계를 추가합니다.

Coordinates are normalized against each device’s current screen. `(0, 0)` is the top left and `(1, 1)` is the bottom right. A job supports up to 32 steps and 60 seconds of explicit waits. Add a wait when a website or app needs extra time to load.

## PowerShell 예제 / PowerShell example

앱에서 **API 토큰 복사**를 누른 직후 실행합니다. / Run immediately after choosing **Copy API token**.

```powershell
$base = 'http://127.0.0.1:17653'
$headers = @{ Authorization = 'Bearer ' + (Get-Clipboard -Raw).Trim() }

Invoke-RestMethod "$base/v1/devices" -Headers $headers
Invoke-RestMethod "$base/v1/devices/2/apps" -Headers $headers

$body = @{
    targets = 2
    steps = @(
        @{ action = 'open_url'; url = 'https://example.com/' }
        @{ action = 'wait'; ms = 1000 }
        @{ action = 'key'; key = 'back' }
    )
} | ConvertTo-Json -Depth 6

$job = Invoke-RestMethod "$base/v1/actions" -Method Post -Headers $headers -ContentType 'application/json' -Body $body
Invoke-RestMethod "$base/v1/jobs/$($job.job_id)" -Headers $headers
```

각 기기에서 단계는 순서대로 실행됩니다. 한 기기에서 단계가 실패하면 해당 기기의 후속 단계는 중단되고 다른 기기의 작업은 계속됩니다. 작업이 끝나면 `succeeded`, `partial_failure`, `failed` 중 하나와 기기별 `completed_steps`가 반환됩니다. 실패한 기기의 `failed_step`은 1부터 시작합니다. 앱과 API는 같은 실행 엔진을 사용합니다. 한 작업이 실행 중일 때 새 작업을 보내면 HTTP 409를 반환합니다.

Steps run in order on each device. A failure stops the remaining steps on that device while other devices continue. Completion returns `succeeded`, `partial_failure`, or `failed` and per-device `completed_steps`. A failed device includes a one-based `failed_step`. The app and API share one job runner. Submitting a job while another is running returns HTTP 409.

## 저장한 프리셋 실행 및 재시도 / Run and retry saved presets

**작업 프리셋** 화면에서 동작을 저장한 뒤, `GET /v1/presets`로 ID를 조회합니다. `/run` 요청 본문에 `targets`를 넣으면 기존 작업과 같은 HTTP 202 응답을 받습니다. 태그가 일치하는 기기 중 연결되어 있고 제어가 가능한 기기가 실행 대상입니다.

Create a preset in **Job presets**, then look up its ID with `GET /v1/presets`. Send `targets` in the `/run` body to receive the standard HTTP 202 job response. For a tag, only connected controllable devices with an exact match are included.

```powershell
Invoke-RestMethod "$base/v1/presets" -Headers $headers
$body = @{ targets = @{ tag = 'QA' } } | ConvertTo-Json
$job = Invoke-RestMethod "$base/v1/presets/1/run" -Method Post -Headers $headers -ContentType 'application/json' -Body $body
Invoke-RestMethod "$base/v1/jobs/$($job.job_id)" -Headers $headers

# Run this after the original job finishes with failures.
$retry = Invoke-RestMethod "$base/v1/jobs/$($job.job_id)/retry" -Method Post -Headers $headers
Invoke-RestMethod "$base/v1/jobs/$($retry.job_id)" -Headers $headers
```

재시도 요청에는 본문이 필요 없습니다. 실패했던 동일 기기에서 원래 단계를 첫 단계부터 실행하고 새 작업 ID를 반환합니다. 실패한 기기 중 제어 가능한 기기가 한 대도 없으면 HTTP 409를 반환합니다. 작업 기록은 앱을 닫기 전까지 최근 32개를 볼 수 있고, 저장한 프리셋은 앱을 다시 열어도 남습니다.

Retry takes an empty body. It replays the original steps from step one on the same devices that failed and returns a new job ID. If none of the failed devices is available for control, the API returns HTTP 409. The app keeps the latest 32 jobs for the current session. Saved presets persist across restarts.
