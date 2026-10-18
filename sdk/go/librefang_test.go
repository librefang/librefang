package librefang

import (
	"errors"
	"io"
	"net/http"
	"strings"
	"testing"
)

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(req *http.Request) (*http.Response, error) {
	return f(req)
}

func TestStreamReportsBodyMarshalErrorBeforeNetwork(t *testing.T) {
	networkCalled := false
	client := New("http://daemon.invalid")
	client.HTTP.Transport = roundTripFunc(func(*http.Request) (*http.Response, error) {
		networkCalled = true
		return nil, errors.New("network must not be reached")
	})

	events := client.stream("POST", "/events", map[string]interface{}{
		"unsupported": func() {},
	}, nil)
	event, ok := <-events
	if !ok {
		t.Fatal("stream closed without reporting the marshal error")
	}
	if got, want := event["status"], 0; got != want {
		t.Fatalf("status = %#v, want %#v", got, want)
	}
	errorText, ok := event["error"].(string)
	if !ok || !strings.Contains(errorText, "marshal: json: unsupported type: func()") {
		t.Fatalf("error = %#v, want body marshal failure", event["error"])
	}
	if _, ok := <-events; ok {
		t.Fatal("stream emitted more than one marshal error")
	}
	if networkCalled {
		t.Fatal("HTTP transport was called after body marshaling failed")
	}
}

func TestStreamReportsRequestConstructionError(t *testing.T) {
	client := New("http://daemon.invalid")
	events := client.stream("GET\nInjected: yes", "/events", nil, nil)

	event, ok := <-events
	if !ok {
		t.Fatal("stream closed without reporting the request construction error")
	}
	if got, want := event["status"], 0; got != want {
		t.Fatalf("status = %#v, want %#v", got, want)
	}
	errorText, ok := event["error"].(string)
	if !ok || !strings.Contains(errorText, "new request: net/http: invalid method") {
		t.Fatalf("error = %#v, want invalid-method construction failure", event["error"])
	}
	if _, ok := <-events; ok {
		t.Fatal("stream emitted more than one request construction error")
	}
}

func TestPathParametersArePercentEncodedAsOneSegment(t *testing.T) {
	var gotPath string
	client := New("http://daemon.invalid")
	client.HTTP.Transport = roundTripFunc(func(req *http.Request) (*http.Response, error) {
		gotPath = req.URL.EscapedPath()
		if req.URL.RawQuery != "" {
			t.Fatalf("a path parameter leaked into the query string: %q", req.URL.RawQuery)
		}
		return &http.Response{
			StatusCode: 200,
			Body:       io.NopCloser(strings.NewReader("{}")),
			Header:     make(http.Header),
		}, nil
	})

	if _, err := client.Memory.GetAgentKvKey("agent 1", "prefs?v=2/../x"); err != nil {
		t.Fatalf("request failed: %v", err)
	}
	if want := "/api/memory/agents/agent%201/kv/prefs%3Fv=2%2F..%2Fx"; gotPath != want {
		t.Fatalf("path = %q, want %q", gotPath, want)
	}
}
