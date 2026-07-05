;;; lisplens-parinfer.el --- Parinfer for Lisp via the lisplens server -*- lexical-binding: t; -*-

;; Copyright (C) 2026  USAMI Kenta

;; Author: USAMI Kenta <tadsan@zonu.me>
;; URL: https://github.com/rigortype/lisplens
;; Version: 0.1.0
;; Package-Requires: ((emacs "29.1"))
;; Keywords: languages, lisp, tools

;; This program is free software; you can redistribute it and/or modify
;; it under the terms of the Mozilla Public License, v. 2.0.  If a copy of
;; the MPL was not distributed with this file, You can obtain one at
;; https://mozilla.org/MPL/2.0/.

;;; Commentary:

;; A native Emacs front-end for lisplens's parinfer command
;; (`lisplens parinfer`).  It is *not* a fork of parinfer-rust-mode and keeps
;; no compatibility with it: lisplens is a CLI, so this talks to one long-lived
;; `lisplens parinfer --server' process (a line-delimited JSON protocol) shared
;; across every buffer, instead of loading a dynamic module.
;;
;; It provides the explicit commands `lisplens-parinfer-paren' and
;; `lisplens-parinfer-indent', and a minor mode `lisplens-parinfer-mode' that
;; runs one of them **live** — reflowing after each edit (debounced) so parens
;; and indentation stay in sync as you type.
;;
;;   paren  — parens are the source of truth; indentation is corrected.
;;   indent — indentation is the source of truth; close-parens are inferred.
;;
;; Live mode uses indent (`lisplens-parinfer-live-mode') because it handles the
;; unbalanced input that is normal mid-edit; the server's cursor-line protection
;; keeps the trail on point's line from collapsing under you.
;;
;; On the server's refusal (unbalanced input, an unterminated string, …) the
;; buffer is left untouched and the diagnostic is echoed.

;;; Code:

(require 'subr-x)

(defgroup lisplens-parinfer nil
  "Parinfer for Lisp, backed by the lisplens server."
  :group 'tools
  :prefix "lisplens-parinfer-")

(defcustom lisplens-parinfer-executable "lisplens"
  "The lisplens executable (found on `exec-path' or an absolute path)."
  :type 'string)

(defcustom lisplens-parinfer-timeout 2.0
  "Seconds to wait for the server to answer one request before giving up."
  :type 'number)

(defcustom lisplens-parinfer-dialect-alist
  '((emacs-lisp-mode . "emacs-lisp")
    (lisp-interaction-mode . "emacs-lisp")
    (lisp-data-mode . "emacs-lisp")
    (lisp-mode . "common-lisp")
    (clojure-mode . "clojure")
    (clojurescript-mode . "clojure")
    (clojurec-mode . "clojure")
    (clojure-ts-mode . "clojure")
    (scheme-mode . "scheme")
    (racket-mode . "racket")
    (fennel-mode . "fennel")
    (janet-mode . "janet")
    (hy-mode . "hy")
    (lfe-mode . "lfe")
    (phel-mode . "phel"))
  "Map a major mode to the lisplens dialect name sent to the server.
When the current major mode (or one of its parents) is absent, no dialect is
sent and the server defaults to Emacs Lisp."
  :type '(alist :key-type symbol :value-type string))

(defcustom lisplens-parinfer-nameless 'auto
  "Whether to enable the Nameless overlay (Emacs Lisp only).
`auto' enables it when `nameless-mode' is active in the buffer; t forces it on,
nil off."
  :type '(choice (const :tag "Follow nameless-mode" auto)
                 (const :tag "On" t)
                 (const :tag "Off" nil)))

(defcustom lisplens-parinfer-live-mode 'indent
  "Which mode `lisplens-parinfer-mode' runs live, on every edit.
`indent' is the sensible choice: it handles the unbalanced input that is
normal while typing.  `paren' requires balanced parens, so it refuses (does
nothing) whenever the buffer is mid-edit — offered only for completeness."
  :type '(choice (const :tag "Indent" indent)
                 (const :tag "Paren" paren)))

(defcustom lisplens-parinfer-idle-delay 0.05
  "Idle seconds to wait before running a live transform after an edit.
Coalesces bursts of fast typing into one server round-trip."
  :type 'number)

;;; Server process ----------------------------------------------------------

(defvar lisplens-parinfer--process nil
  "The shared `lisplens parinfer --server' process, or nil.")

(defvar lisplens-parinfer--output ""
  "Accumulated stdout from the server for the in-flight request.")

(defun lisplens-parinfer--filter (_proc string)
  "Collect STRING from the server into `lisplens-parinfer--output'."
  (setq lisplens-parinfer--output (concat lisplens-parinfer--output string)))

(defun lisplens-parinfer--ensure-process ()
  "Start the shared server process if it is not already running."
  (unless (process-live-p lisplens-parinfer--process)
    (setq lisplens-parinfer--output "")
    (setq lisplens-parinfer--process
          (make-process
           :name "lisplens-parinfer"
           :command (list lisplens-parinfer-executable "parinfer" "--server")
           :connection-type 'pipe
           :coding 'utf-8-unix
           :noquery t
           :filter #'lisplens-parinfer--filter
           :stderr (get-buffer-create " *lisplens-parinfer-stderr*"))))
  lisplens-parinfer--process)

(defun lisplens-parinfer-restart ()
  "Stop the shared server process; the next request starts a fresh one."
  (interactive)
  (when (process-live-p lisplens-parinfer--process)
    (delete-process lisplens-parinfer--process))
  (setq lisplens-parinfer--process nil))

(defun lisplens-parinfer--request (request)
  "Send REQUEST (an alist/plist) to the server and return the parsed answer.
Blocks until one answer line arrives or `lisplens-parinfer-timeout' elapses."
  (lisplens-parinfer--ensure-process)
  (setq lisplens-parinfer--output "")
  (process-send-string lisplens-parinfer--process
                       (concat (json-serialize request) "\n"))
  (let ((deadline (+ (float-time) lisplens-parinfer-timeout)))
    (while (and (not (string-search "\n" lisplens-parinfer--output))
                (< (float-time) deadline)
                (process-live-p lisplens-parinfer--process))
      (accept-process-output lisplens-parinfer--process 0.05)))
  (let ((nl (string-search "\n" lisplens-parinfer--output)))
    (unless nl
      (error "lisplens-parinfer: no response from server"))
    (json-parse-string (substring lisplens-parinfer--output 0 nl)
                       :object-type 'alist :null-object nil :false-object :false)))

;;; Request/answer ----------------------------------------------------------

(defun lisplens-parinfer--dialect ()
  "The lisplens dialect for the current buffer, or nil to let the server decide."
  (seq-some (lambda (cell)
              (and (derived-mode-p (car cell)) (cdr cell)))
            lisplens-parinfer-dialect-alist))

(defun lisplens-parinfer--nameless-p ()
  "Whether to send the Nameless overlay for the current buffer."
  (pcase lisplens-parinfer-nameless
    ('auto (bound-and-true-p nameless-mode))
    (val val)))

(defun lisplens-parinfer--bounds ()
  "The line-aligned (BEG . END) to transform: the active region, else the buffer.
Region bounds are widened to whole lines so the sent text starts at a line
beginning, keeping the cursor line/column arithmetic exact."
  (if (use-region-p)
      (cons (save-excursion (goto-char (region-beginning)) (line-beginning-position))
            (save-excursion (goto-char (region-end)) (line-end-position)))
    (cons (point-min) (point-max))))

(defun lisplens-parinfer--run (mode &optional quiet)
  "Transform the buffer (or region) with MODE (`paren' or `indent').
With QUIET, a refusal is silent (no echo) — used by live firing, which would
otherwise spam the echo area on every keystroke of an in-progress string."
  (let* ((bounds (lisplens-parinfer--bounds))
         (beg (car bounds))
         (end (cdr bounds))
         (text (buffer-substring-no-properties beg end))
         (in-scope (<= beg (point) end))
         (dialect (lisplens-parinfer--dialect))
         (request
          (append
           (list :mode (symbol-name mode) :text text)
           (when dialect (list :dialect dialect))
           (when (and (equal dialect "emacs-lisp") (lisplens-parinfer--nameless-p))
             (list :nameless t :name (or (buffer-file-name) "")))
           (when in-scope
             (list :cursorLine (- (line-number-at-pos (point))
                                  (line-number-at-pos beg))
                   :cursorX (- (point) (line-beginning-position))))))
         (answer (lisplens-parinfer--request request)))
    (lisplens-parinfer--apply answer beg end quiet)))

(defun lisplens-parinfer--apply (answer beg end &optional quiet)
  "Apply ANSWER to the region BEG..END, or (unless QUIET) report its diagnostic."
  (if (not (eq (alist-get 'success answer) t))
      (unless quiet
        (let ((err (alist-get 'error answer)))
          (message "lisplens-parinfer: %s (%s)"
                   (or (alist-get 'message err) "refused")
                   (or (alist-get 'name err) "error"))))
    (let ((text (alist-get 'text answer))
          (cline (alist-get 'cursorLine answer))
          (cx (alist-get 'cursorX answer)))
      (unless (string= text (buffer-substring-no-properties beg end))
        ;; Minimal-diff replace preserves markers and keeps undo tight.
        (replace-region-contents beg end (lambda () text)))
      (when (and (numberp cline) (numberp cx))
        (goto-char beg)
        (forward-line cline)
        (goto-char (min (+ (point) cx) (line-end-position)))))))

;;; Commands ----------------------------------------------------------------

;;;###autoload
(defun lisplens-parinfer-paren ()
  "Run parinfer paren mode on the buffer (or active region)."
  (interactive)
  (lisplens-parinfer--run 'paren))

;;;###autoload
(defun lisplens-parinfer-indent ()
  "Run parinfer indent mode on the buffer (or active region)."
  (interactive)
  (lisplens-parinfer--run 'indent))

(defvar-keymap lisplens-parinfer-mode-map
  :doc "Keymap for `lisplens-parinfer-mode'."
  "C-c C-p p" #'lisplens-parinfer-paren
  "C-c C-p i" #'lisplens-parinfer-indent)

;;; Live firing -------------------------------------------------------------

;; Forward declaration: the variable is created by the `define-minor-mode' below.
(defvar lisplens-parinfer-mode)

(defvar-local lisplens-parinfer--tick nil
  "`buffer-chars-modified-tick' at the last live fire, to skip no-op commands.")

(defvar-local lisplens-parinfer--timer nil
  "Pending idle timer that will run the live transform for this buffer.")

(defun lisplens-parinfer--fire (buffer)
  "Run the live transform in BUFFER if it is still live and modified."
  (when (buffer-live-p buffer)
    (with-current-buffer buffer
      (setq lisplens-parinfer--timer nil)
      (when lisplens-parinfer-mode
        ;; A refusal (mid-edit unbalanced input) is silent; never let a server
        ;; hiccup break the command loop.
        (with-demoted-errors "lisplens-parinfer: %S"
          (lisplens-parinfer--run lisplens-parinfer-live-mode t))
        (setq lisplens-parinfer--tick (buffer-chars-modified-tick))))))

(defun lisplens-parinfer--post-command ()
  "Schedule a debounced live transform when this command changed the buffer."
  (when (/= (buffer-chars-modified-tick) (or lisplens-parinfer--tick -1))
    (setq lisplens-parinfer--tick (buffer-chars-modified-tick))
    (when (timerp lisplens-parinfer--timer)
      (cancel-timer lisplens-parinfer--timer))
    (setq lisplens-parinfer--timer
          (run-with-idle-timer lisplens-parinfer-idle-delay nil
                               #'lisplens-parinfer--fire (current-buffer)))))

(defun lisplens-parinfer--any-buffer-p ()
  "Whether any live buffer still has `lisplens-parinfer-mode' on."
  (seq-some (lambda (b) (buffer-local-value 'lisplens-parinfer-mode b))
            (buffer-list)))

;;;###autoload
(define-minor-mode lisplens-parinfer-mode
  "Live parinfer for the current buffer, backed by the lisplens server.

While on, every edit is reflowed by `lisplens-parinfer-live-mode' (indent by
default) after a short idle delay, keeping parens and indentation in sync as you
type; the cursor line's paren trail is left alone so it does not fight you.  The
explicit commands `lisplens-parinfer-paren' / `lisplens-parinfer-indent' stay
available under \\`C-c C-p'."
  :lighter " Parinfer"
  :keymap lisplens-parinfer-mode-map
  (if lisplens-parinfer-mode
      (progn
        (setq lisplens-parinfer--tick (buffer-chars-modified-tick))
        (add-hook 'post-command-hook #'lisplens-parinfer--post-command nil t))
    (remove-hook 'post-command-hook #'lisplens-parinfer--post-command t)
    (when (timerp lisplens-parinfer--timer)
      (cancel-timer lisplens-parinfer--timer)
      (setq lisplens-parinfer--timer nil))
    ;; Stop the shared process once nothing uses it.
    (unless (lisplens-parinfer--any-buffer-p)
      (lisplens-parinfer-restart))))

(provide 'lisplens-parinfer)
;;; lisplens-parinfer.el ends here
