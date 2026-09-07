// Package server exposes the fixture HTTP surface.
package server

import (
	"errors"
	"fmt"
)

// ErrNotFound is returned when a route has no handler.
var ErrNotFound = errors.New("not found")

// MaxRoutes bounds the fixture router.
const MaxRoutes = 32

// Router dispatches requests to handlers.
type Router struct {
	routes map[string]string
}

// Handler responds to a single route.
type Handler interface {
	Serve(path string) (string, error)
}

// NewRouter builds an empty router.
func NewRouter() *Router {
	return &Router{routes: map[string]string{}}
}

// Handle registers one route.
func (r *Router) Handle(path string, target string) error {
	if len(r.routes) >= MaxRoutes {
		return fmt.Errorf("router full: %w", ErrNotFound)
	}
	r.routes[path] = target
	return nil
}
