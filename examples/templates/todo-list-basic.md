# Basic Todo List Application

> Create a simple, functional CRUD todo list application.

## Context
The application should be a standard web application with a frontend and a backend API. For simplicity, use an in-memory store or a simple file-based store for todos. Do not use a full database like PostgreSQL or MongoDB.

## Tasks

### T1: Project Setup & Basic Structure
Create a new project directory. Set up a monorepo structure or a standard frontend/backend split (e.g., `frontend/` and `backend/`). Initialize package managers (`package.json` for Node.js backend, `package.json` for frontend). Add basic linters/formatters (`eslint`, `prettier`).

### T2: Backend API (Node.js/Express)
- Create a simple Express.js backend server.
- Implement basic API endpoints for CRUD operations on todos:
    - `POST /todos` (create new todo)
    - `GET /todos` (list all todos)
    - `PUT /todos/:id` (update todo, e.g., mark as complete)
    - `DELETE /todos/:id` (delete todo)
- Use an in-memory array to store todos (no database).
- Ensure API responses are JSON.

### T3: Frontend (SvelteKit/Vite)
- Create a frontend using **SvelteKit with Vite**.
- Fetch and display the list of todos from the backend API.
- Implement UI elements for:
    - Adding a new todo (input field + button).
    - Marking a todo as complete/incomplete (checkbox or button).
    - Deleting a todo.
- Ensure the frontend communicates with the backend API.

### T4: Basic Styling
Apply minimal styling using CSS or a CSS framework (e.g., Tailwind CSS) to make the app presentable.

### T5: README & Project Structure
Create a `README.md` file explaining how to run the application (backend + frontend setup, start commands).
Ensure the project directory structure is clean and organized.

### T6: Add Basic Tests
Write a few unit tests for the backend API logic AND integration tests for the full flow (frontend interacting with backend). Aim for basic coverage.
