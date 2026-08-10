package librefang

import (
	"errors"
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
